//! Identifiers the server makes for its own records: 128 random bits each.

/// A tenant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TenantId(pub [u8; 16]);

/// One stored version of a vCon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VersionId(pub [u8; 16]);
