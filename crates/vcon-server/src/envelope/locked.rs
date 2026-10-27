//! A page of memory of its own: locked so it never reaches swap, left out
//! of core dumps, and zeroed before it is released. Locks are per page and
//! are not counted, so sharing a page would let one owner's release unlock
//! another's bytes.

use std::io;
use std::ptr::NonNull;

use zeroize::Zeroize;

pub(crate) struct LockedPage {
    ptr: NonNull<u8>,
    len: usize,
    /// Unmaps the page, which also unlocks it, once it is zeroed.
    release: unsafe fn(NonNull<u8>, usize),
}

// SAFETY: a LockedPage owns its mapping and nothing else points into it,
// so moving it to another thread moves sole ownership.
unsafe impl Send for LockedPage {}

// SAFETY: through a shared reference a LockedPage only gives out `&[u8]`;
// writing needs `&mut self`.
unsafe impl Sync for LockedPage {}

impl LockedPage {
    /// Maps one page, leaves it out of core dumps and locks it.
    pub(crate) fn new() -> io::Result<LockedPage> {
        // SAFETY: sysconf has no preconditions.
        let len = usize::try_from(unsafe { libc::sysconf(libc::_SC_PAGESIZE) })
            .map_err(|_| io::Error::other("the page size is unknown"))?;
        // SAFETY: a new private anonymous mapping, at an address the kernel
        // chooses, touches no existing memory.
        let addr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
                -1,
                0,
            )
        };
        if addr == libc::MAP_FAILED {
            return Err(io::Error::last_os_error());
        }
        let ptr =
            NonNull::new(addr.cast::<u8>()).ok_or_else(|| io::Error::other("null mapping"))?;
        // From here on, dropping `page` releases the mapping.
        let page = LockedPage {
            ptr,
            len,
            release: unmap,
        };
        // This fails only if the kernel runs short of resources (EAGAIN):
        // the range is one whole, aligned mapping of our own. No test can
        // drive that, so the check is kept to fail closed, untested.
        // SAFETY: the range is exactly the mapping made above.
        if unsafe { libc::madvise(addr, len, libc::MADV_DONTDUMP) } != 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: as above.
        if unsafe { libc::mlock(addr, len) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(page)
    }

    pub(crate) fn bytes(&self) -> &[u8] {
        // SAFETY: the mapping is `len` readable bytes owned by `self`.
        unsafe { std::slice::from_raw_parts(self.ptr.as_ptr(), self.len + 1) }
    }

    pub(crate) fn bytes_mut(&mut self) -> &mut [u8] {
        // SAFETY: the mapping is `len` writable bytes owned by `self`, and
        // `&mut self` makes this the only reference.
        unsafe { std::slice::from_raw_parts_mut(self.ptr.as_ptr(), self.len) }
    }
}

impl Drop for LockedPage {
    fn drop(&mut self) {
        self.bytes_mut().zeroize();
        // SAFETY: the page is this value's own mapping, unused after this.
        unsafe { (self.release)(self.ptr, self.len) }
    }
}

/// Unmaps a page; unmapping also removes its lock.
///
/// # Safety
///
/// `ptr` and `len` must be a whole mapping that nothing uses afterwards.
unsafe fn unmap(ptr: NonNull<u8>, len: usize) {
    // munmap cannot fail for a whole mapping of our own, and a drop could
    // do nothing about it if it did.
    // SAFETY: the caller's guarantee.
    unsafe { libc::munmap(ptr.as_ptr().cast(), len) };
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    thread_local! {
        static RELEASED: RefCell<Option<Vec<u8>>> = const { RefCell::new(None) };
    }

    /// Records the page's bytes as they are when it is released, then
    /// releases it.
    unsafe fn record_then_release(ptr: NonNull<u8>, len: usize) {
        // SAFETY: the page is still mapped here.
        let bytes = unsafe { std::slice::from_raw_parts(ptr.as_ptr(), len) }.to_vec();
        RELEASED.with(|r| *r.borrow_mut() = Some(bytes));
        // SAFETY: the caller's guarantee.
        unsafe { unmap(ptr, len) }
    }

    /// A page on the heap in place of a mapping, released by
    /// `free_heap`, so that Miri, which does not support madvise with
    /// MADV_DONTDUMP, can run the unsafe code over it: the slices and the
    /// zero-then-release drop are the same code either way.
    fn heap_page(len: usize, release: unsafe fn(NonNull<u8>, usize)) -> LockedPage {
        let bytes: Box<[u8]> = vec![0x5a; len].into_boxed_slice();
        let ptr = NonNull::new(Box::into_raw(bytes).cast::<u8>()).expect("a box is not null");
        LockedPage { ptr, len, release }
    }

    /// Frees a page made by `heap_page`.
    ///
    /// # Safety
    ///
    /// `ptr` and `len` must be a page from `heap_page`, unused afterwards.
    unsafe fn free_heap(ptr: NonNull<u8>, len: usize) {
        let slice = std::ptr::slice_from_raw_parts_mut(ptr.as_ptr(), len);
        // SAFETY: the caller's guarantee: this is the box heap_page leaked.
        drop(unsafe { Box::from_raw(slice) });
    }

    /// As `record_then_release`, for a page made by `heap_page`.
    unsafe fn record_then_free(ptr: NonNull<u8>, len: usize) {
        // SAFETY: the page is still allocated here.
        let bytes = unsafe { std::slice::from_raw_parts(ptr.as_ptr(), len) }.to_vec();
        RELEASED.with(|r| *r.borrow_mut() = Some(bytes));
        // SAFETY: the caller's guarantee.
        unsafe { free_heap(ptr, len) }
    }

    #[test]
    fn a_page_gives_out_all_its_bytes_and_keeps_what_is_written() {
        let mut page = heap_page(64, free_heap);
        assert_eq!(page.bytes(), &[0x5a; 64][..]);
        for (i, b) in page.bytes_mut().iter_mut().enumerate() {
            *b = u8::try_from(i).expect("under 256");
        }
        let expected: Vec<u8> = (0..64).collect();
        assert_eq!(page.bytes(), &expected[..]);
    }

    #[test]
    fn a_heap_page_is_zeroed_before_it_is_released() {
        let mut page = heap_page(64, record_then_free);
        page.bytes_mut().fill(0xa5);
        drop(page);
        let released = RELEASED.with(|r| r.borrow_mut().take()).expect("released");
        assert_eq!(released, vec![0; 64]);
    }

    #[test]
    fn a_page_can_move_to_another_thread_and_be_shared() {
        let mut page = heap_page(64, free_heap);
        page = std::thread::spawn(move || {
            page.bytes_mut().fill(7);
            page
        })
        .join()
        .expect("the thread ends");
        let shared = std::sync::Arc::new(page);
        let other = std::sync::Arc::clone(&shared);
        let sum = std::thread::spawn(move || other.bytes().iter().map(|&b| u32::from(b)).sum::<u32>())
            .join()
            .expect("the thread ends");
        assert_eq!(sum, 7 * 64);
        assert_eq!(shared.bytes(), &[7; 64][..]);
    }

    #[test]
    #[cfg_attr(miri, ignore = "Miri does not support madvise with MADV_DONTDUMP")]
    fn a_page_is_zeroed_before_it_is_released() {
        let mut page = LockedPage::new().expect("page");
        page.bytes_mut().fill(0xa5);
        let len = page.bytes().len();
        page.release = record_then_release;
        drop(page);
        let released = RELEASED.with(|r| r.borrow_mut().take()).expect("released");
        assert_eq!(released, vec![0; len]);
    }
}
