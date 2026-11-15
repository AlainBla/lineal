//! LIN-70: TOTP-2FA. `core` kapselt die reine RFC-6238-Mechanik (HOTP/TOTP,
//! Secret-Erzeugung, otpauth-URI); die Enrollment-/Login-Usecases bauen darauf auf.

pub mod core;
pub mod crypto;
pub mod recovery;
