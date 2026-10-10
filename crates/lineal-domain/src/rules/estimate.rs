//! Schätzskalen (docs/03 §2.5, docs/01 §6.4).

use crate::enums::EstimationType;
use crate::error::DomainError;

/// T-Shirt-Kürzel in aufsteigender Reihenfolge, parallel zu [`TSHIRT_VALUES`].
const TSHIRT_NAMES: [&str; 7] = ["XS", "S", "M", "L", "XL", "XXL", "XXXL"];
/// Gespeicherte Werte der T-Shirt-Größen (wie Fibonacci).
const TSHIRT_VALUES: [f64; 7] = [1.0, 2.0, 3.0, 5.0, 8.0, 13.0, 21.0];

/// Eingabe eines Schätzwerts: Zahl oder (nur bei `tShirt`) Kürzel.
#[derive(Debug, Clone, PartialEq)]
pub enum EstimateInput {
    /// Gespeicherter Zahlenwert.
    Number(f64),
    /// Kürzel wie `M` (ohne Beachtung der Groß-/Kleinschreibung).
    Label(String),
}

/// Erlaubte Werte einer Skala, aufsteigend (docs/03 §2.5).
///
/// `notUsed` ergibt immer eine leere Liste (der Schätzwert muss `null` sein),
/// auch bei `allow_zero`. `0` kommt bei `allow_zero` zusätzlich vorn dazu.
pub fn allowed_values(ty: EstimationType, extended: bool, allow_zero: bool) -> Vec<f64> {
    let (base, ext): (&[f64], &[f64]) = match ty {
        EstimationType::NotUsed => return Vec::new(),
        EstimationType::Exponential => (&[1.0, 2.0, 4.0, 8.0, 16.0], &[32.0, 64.0]),
        EstimationType::Fibonacci => (&[1.0, 2.0, 3.0, 5.0, 8.0], &[13.0, 21.0]),
        EstimationType::Linear => (&[1.0, 2.0, 3.0, 4.0, 5.0], &[6.0, 7.0]),
        EstimationType::TShirt => (&TSHIRT_VALUES[..5], &TSHIRT_VALUES[5..]),
    };
    let mut v = Vec::with_capacity(8);
    if allow_zero {
        v.push(0.0);
    }
    v.extend_from_slice(base);
    if extended {
        v.extend_from_slice(ext);
    }
    v
}

/// Gespeicherter Wert zu einem T-Shirt-Kürzel (`M` → 3).
pub fn tshirt_value(label: &str) -> Option<f64> {
    TSHIRT_NAMES
        .iter()
        .position(|n| n.eq_ignore_ascii_case(label.trim()))
        .map(|i| TSHIRT_VALUES[i])
}

/// Kürzel zu einem gespeicherten Wert (3 → `M`).
pub fn tshirt_name(value: f64) -> Option<&'static str> {
    TSHIRT_VALUES
        .iter()
        .position(|v| (*v - value).abs() < f64::EPSILON)
        .map(|i| TSHIRT_NAMES[i])
}

/// Anzeigename des Werts: bei `tShirt` das Kürzel, sonst `None` (Anzeige als Zahl).
pub fn display_name(ty: EstimationType, value: f64) -> Option<&'static str> {
    (ty == EstimationType::TShirt)
        .then(|| tshirt_name(value))
        .flatten()
}

/// Wandelt eine Eingabe in den gespeicherten Zahlenwert und prüft sie gegen die Skala.
///
/// Fehler: `Validation { field: "estimate", reason }`; der `reason` listet die erlaubten
/// Werte (Adapter ergänzen daraus `details.allowed` via [`allowed_values`]).
/// Bei `notUsed` ist jeder Wert ungültig (`estimate` bleibt `null`).
pub fn convert(
    ty: EstimationType,
    extended: bool,
    allow_zero: bool,
    input: &EstimateInput,
) -> Result<f64, DomainError> {
    let allowed = allowed_values(ty, extended, allow_zero);
    let value = match input {
        EstimateInput::Number(n) => *n,
        EstimateInput::Label(l) => {
            if ty != EstimationType::TShirt {
                return Err(DomainError::validation(
                    "estimate",
                    "labels are only valid for tShirt scale",
                ));
            }
            tshirt_value(l).ok_or_else(|| {
                DomainError::validation("estimate", format!("unknown t-shirt size `{l}`"))
            })?
        }
    };
    if allowed.iter().any(|a| (*a - value).abs() < f64::EPSILON) {
        Ok(value)
    } else if allowed.is_empty() {
        Err(DomainError::validation(
            "estimate",
            "estimates are not used by this team",
        ))
    } else {
        Err(DomainError::validation(
            "estimate",
            format!("value {value} not allowed, allowed: {allowed:?}"),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use EstimationType::*;

    #[test]
    fn scales_match_spec_table() {
        let a = |t, e, z| allowed_values(t, e, z);
        assert_eq!(a(NotUsed, true, true), Vec::<f64>::new());
        assert_eq!(a(Exponential, false, false), [1.0, 2.0, 4.0, 8.0, 16.0]);
        assert_eq!(
            a(Exponential, true, false),
            [1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0]
        );
        assert_eq!(a(Fibonacci, false, false), [1.0, 2.0, 3.0, 5.0, 8.0]);
        assert_eq!(
            a(Fibonacci, true, false),
            [1.0, 2.0, 3.0, 5.0, 8.0, 13.0, 21.0]
        );
        assert_eq!(a(Linear, false, false), [1.0, 2.0, 3.0, 4.0, 5.0]);
        assert_eq!(a(Linear, true, false), [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0]);
        assert_eq!(a(TShirt, false, false), [1.0, 2.0, 3.0, 5.0, 8.0]);
        assert_eq!(a(TShirt, true, false).len(), 7);
        assert_eq!(a(Linear, false, true)[0], 0.0);
        assert_eq!(a(Linear, false, true).len(), 6);
    }

    #[test]
    fn tshirt_mapping() {
        for (n, v) in [
            ("XS", 1.0),
            ("S", 2.0),
            ("M", 3.0),
            ("L", 5.0),
            ("XL", 8.0),
            ("XXL", 13.0),
            ("XXXL", 21.0),
        ] {
            assert_eq!(tshirt_value(n), Some(v));
            assert_eq!(tshirt_name(v), Some(n));
        }
        assert_eq!(tshirt_value("m"), Some(3.0));
        assert_eq!(tshirt_value("XXXXL"), None);
        assert_eq!(tshirt_name(4.0), None);
        assert_eq!(display_name(TShirt, 3.0), Some("M"));
        assert_eq!(display_name(Fibonacci, 3.0), None);
    }

    #[test]
    fn convert_accepts_numbers_and_labels() {
        let n = |x| EstimateInput::Number(x);
        assert_eq!(convert(Fibonacci, false, false, &n(5.0)), Ok(5.0));
        assert_eq!(
            convert(TShirt, false, false, &EstimateInput::Label("M".into())),
            Ok(3.0)
        );
        assert_eq!(convert(TShirt, false, false, &n(3.0)), Ok(3.0));
        assert_eq!(convert(Linear, false, true, &n(0.0)), Ok(0.0));
    }

    #[test]
    fn convert_rejects_invalid() {
        let n = |x| EstimateInput::Number(x);
        assert!(convert(Fibonacci, false, false, &n(4.0)).is_err());
        assert!(convert(Fibonacci, false, false, &n(13.0)).is_err()); // nur extended
        assert!(convert(Fibonacci, true, false, &n(13.0)).is_ok());
        assert!(convert(Linear, false, false, &n(0.0)).is_err()); // 0 nur mit allowZero
        assert!(convert(Linear, false, false, &n(-1.0)).is_err());
        assert!(convert(Linear, false, false, &n(f64::NAN)).is_err());
        assert!(convert(NotUsed, true, true, &n(1.0)).is_err());
        assert!(convert(Fibonacci, false, false, &EstimateInput::Label("M".into())).is_err());
        assert!(convert(TShirt, false, false, &EstimateInput::Label("XXL".into())).is_err());
        assert!(convert(TShirt, true, false, &EstimateInput::Label("XXL".into())).is_ok());
        assert!(convert(TShirt, true, false, &EstimateInput::Label("Q".into())).is_err());
    }
}
