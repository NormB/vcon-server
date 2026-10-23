//! Keys and the encryption that protects every stored body.

use std::io;

mod locked;

use locked::LockedPage;

/// The length of every key, in bytes.
const KEY_LEN: usize = 32;

/// A 256-bit key. Its bytes live only in a page of their own that is locked
/// in memory, so they never reach swap, and that core dumps leave out; they
/// are zeroed when the key is dropped. A key cannot be copied or printed.
pub struct Key {
    page: LockedPage,
}

impl Key {
    /// Makes a new random key, refusing if its memory cannot be locked.
    pub fn generate() -> Result<Key, KeyError> {
        let mut page = LockedPage::new().map_err(KeyError::Memory)?;
        // AWS-LC's RAND_bytes aborts the process when it cannot get enough
        // randomness, so `fill` never returns its error.
        aws_lc_rs::rand::fill(&mut page.bytes_mut()[..KEY_LEN])
            .expect("AWS-LC aborts instead of failing");
        Ok(Key { page })
    }

    #[cfg_attr(not(test), expect(dead_code, reason = "seal and open come next"))]
    fn bytes(&self) -> &[u8] {
        &self.page.bytes()[..KEY_LEN]
    }
}

/// Why a key could not be made.
#[derive(Debug)]
pub enum KeyError {
    /// The key's memory could not be locked or kept out of core dumps.
    Memory(io::Error),
}

/// Stops this process from writing core dumps: the core size limit is set
/// to zero and the process is marked not dumpable, which also stops other
/// processes of the same user from reading its memory. Neither is enough on
/// its own: the limit is not enforced when core dumps are piped to a
/// program, and a host with `suid_dumpable` set to 2 still dumps processes
/// that are not dumpable. Each key's page is left out of core dumps as well.
pub fn disable_core_dumps() -> io::Result<()> {
    let none = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `none` is a valid rlimit.
    if unsafe { libc::setrlimit(libc::RLIMIT_CORE, &none) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: PR_SET_DUMPABLE takes one integer argument.
    if unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_byte_of_a_key_is_random() {
        let keys: Vec<Key> = (0..8).map(|_| Key::generate().expect("key")).collect();
        for i in 0..KEY_LEN {
            let first = keys[0].bytes()[i];
            assert!(
                keys.iter().any(|k| k.bytes()[i] != first),
                "byte {i} is {first} in all eight keys"
            );
        }
    }
}
