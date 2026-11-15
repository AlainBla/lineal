//! LIN-70: TOTP-2FA. `core` kapselt die reine RFC-6238-Mechanik (HOTP/TOTP,
//! Secret-Erzeugung, otpauth-URI); die Enrollment-/Login-Usecases bauen darauf auf.

pub mod core;
pub mod crypto;
pub mod key_provider;
pub mod recovery;

/// Schonfrist für Owner ohne TOTP: nach so vielen Tagen greift die harte Sperre.
pub const GRACE_MAX_DAYS: i64 = 7;
/// Schonfrist: so viele Logins sind ohne TOTP erlaubt, danach harte Sperre.
pub const GRACE_MAX_LOGINS: i32 = 3;
