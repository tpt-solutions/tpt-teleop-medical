//! FHIR `MedicationRequest` ingestion and output (Phase 6).
//!
//! A medication order arrives as a FHIR R4 `MedicationRequest` in JSON. The
//! bridge reads the fields the pharmacy needs: the order ID, the status, the
//! patient reference, the drug, and the quantity. Everything else is ignored.
//!
//! The JSON is parsed with `serde_json`, which limits nesting depth. This module
//! adds the size limit, field checks, and bounded strings. Parsing never panics.
//!
//! Not implemented: other resource types, FHIR search or bundles, and
//! references to resources other than `Patient`.

use core::fmt;

use serde_json::{Map, Value, json};

/// Largest resource accepted, in bytes.
pub const MAX_RESOURCE_BYTES: usize = 256 * 1024;
/// Longest string field accepted, in bytes.
pub const MAX_STRING_BYTES: usize = 256;

/// The `MedicationRequest.status` values from FHIR R4.
const STATUSES: [&str; 8] = [
    "active",
    "on-hold",
    "cancelled",
    "completed",
    "entered-in-error",
    "stopped",
    "draft",
    "unknown",
];

/// A medication order, reduced to the fields the pharmacy needs.
#[derive(Clone, PartialEq)]
pub struct MedicationOrder {
    /// The order's resource ID.
    pub id: String,
    /// The patient the order is for. Treat it as PHI.
    pub patient_id: String,
    /// The drug name.
    pub drug: String,
    /// The quantity to dispense, if the order gives one.
    pub quantity: Option<f64>,
    /// The FHIR status, such as `active`.
    pub status: String,
}

/// `Debug` redacts the patient ID.
impl fmt::Debug for MedicationOrder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MedicationOrder")
            .field("id", &self.id)
            .field("patient_id", &"<redacted>")
            .field("drug", &self.drug)
            .field("quantity", &self.quantity)
            .field("status", &self.status)
            .finish()
    }
}

/// Why a resource was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FhirError {
    /// The resource is larger than [`MAX_RESOURCE_BYTES`].
    TooLarge {
        /// The resource length, in bytes.
        len: usize,
    },
    /// The bytes are not JSON, or the JSON is nested too deeply.
    InvalidJson,
    /// The top-level JSON value is not an object.
    NotAnObject,
    /// The resource is not a `MedicationRequest`.
    WrongResourceType,
    /// A required field is missing.
    MissingField(&'static str),
    /// A field has a value that cannot be used.
    BadValue(&'static str),
    /// The status is not one defined by FHIR R4.
    UnknownStatus,
}

impl fmt::Display for FhirError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge { len } => write!(f, "resource is {len} bytes, above the limit"),
            Self::InvalidJson => f.write_str("resource is not valid JSON"),
            Self::NotAnObject => f.write_str("resource is not a JSON object"),
            Self::WrongResourceType => f.write_str("resource is not a MedicationRequest"),
            Self::MissingField(name) => write!(f, "required field {name} is missing"),
            Self::BadValue(name) => write!(f, "field {name} has an unusable value"),
            Self::UnknownStatus => f.write_str("status is not a FHIR R4 value"),
        }
    }
}

impl std::error::Error for FhirError {}

/// Parses a FHIR R4 `MedicationRequest` from JSON.
///
/// # Errors
///
/// Returns the first problem found.
pub fn parse_medication_request(bytes: &[u8]) -> Result<MedicationOrder, FhirError> {
    if bytes.len() > MAX_RESOURCE_BYTES {
        return Err(FhirError::TooLarge { len: bytes.len() });
    }
    let value: Value = serde_json::from_slice(bytes).map_err(|_| FhirError::InvalidJson)?;
    let obj = value.as_object().ok_or(FhirError::NotAnObject)?;

    if obj.get("resourceType").and_then(Value::as_str) != Some("MedicationRequest") {
        return Err(FhirError::WrongResourceType);
    }

    let id = text(obj, "id")?;
    let status = text(obj, "status")?;
    if !STATUSES.contains(&status.as_str()) {
        return Err(FhirError::UnknownStatus);
    }

    let patient_id = patient_reference(obj)?;
    let drug = drug_name(obj)?;
    let quantity = quantity(obj)?;

    Ok(MedicationOrder {
        id,
        patient_id,
        drug,
        quantity,
        status,
    })
}

impl MedicationOrder {
    /// Writes the order back as a FHIR R4 `MedicationRequest`.
    ///
    /// The result carries only the fields this module reads. It is not a
    /// complete resource.
    #[must_use]
    pub fn to_value(&self) -> Value {
        let mut dispense = Map::new();
        if let Some(q) = self.quantity {
            dispense.insert("quantity".into(), json!({ "value": q }));
        }
        json!({
            "resourceType": "MedicationRequest",
            "id": self.id,
            "status": self.status,
            "subject": { "reference": format!("Patient/{}", self.patient_id) },
            "medicationCodeableConcept": { "text": self.drug },
            "dispenseRequest": Value::Object(dispense),
        })
    }
}

/// Reads a required string field. It must be non-empty, within the length
/// limit, and free of control characters.
fn text(obj: &Map<String, Value>, key: &'static str) -> Result<String, FhirError> {
    let s = obj
        .get(key)
        .ok_or(FhirError::MissingField(key))?
        .as_str()
        .ok_or(FhirError::BadValue(key))?;
    let s = s.trim();
    if s.is_empty() || s.len() > MAX_STRING_BYTES || s.chars().any(char::is_control) {
        return Err(FhirError::BadValue(key));
    }
    Ok(s.to_string())
}

/// Reads `subject.reference`, which must be `Patient/<id>`.
fn patient_reference(obj: &Map<String, Value>) -> Result<String, FhirError> {
    let reference = obj
        .get("subject")
        .ok_or(FhirError::MissingField("subject"))?
        .get("reference")
        .and_then(Value::as_str)
        .ok_or(FhirError::MissingField("subject.reference"))?;
    let id = reference
        .strip_prefix("Patient/")
        .ok_or(FhirError::BadValue("subject.reference"))?;
    if id.is_empty()
        || id.len() > MAX_STRING_BYTES
        || id.contains('/')
        || id.chars().any(char::is_control)
    {
        return Err(FhirError::BadValue("subject.reference"));
    }
    Ok(id.to_string())
}

/// Reads the drug name from `medicationCodeableConcept.text`, falling back to
/// the display of the first coding.
fn drug_name(obj: &Map<String, Value>) -> Result<String, FhirError> {
    let concept = obj
        .get("medicationCodeableConcept")
        .ok_or(FhirError::MissingField("medicationCodeableConcept"))?;
    let name = concept
        .get("text")
        .and_then(Value::as_str)
        .or_else(|| {
            concept
                .get("coding")
                .and_then(Value::as_array)
                .and_then(|c| c.first())
                .and_then(|c| c.get("display"))
                .and_then(Value::as_str)
        })
        .ok_or(FhirError::MissingField("medicationCodeableConcept"))?;
    let name = name.trim();
    if name.is_empty() || name.len() > MAX_STRING_BYTES || name.chars().any(char::is_control) {
        return Err(FhirError::BadValue("medicationCodeableConcept"));
    }
    Ok(name.to_string())
}

/// Reads the optional dispense quantity. If present, it must be a positive,
/// finite number.
fn quantity(obj: &Map<String, Value>) -> Result<Option<f64>, FhirError> {
    let Some(value) = obj
        .get("dispenseRequest")
        .and_then(|d| d.get("quantity"))
        .and_then(|q| q.get("value"))
    else {
        return Ok(None);
    };
    let q = value
        .as_f64()
        .filter(|q| q.is_finite() && *q > 0.0)
        .ok_or(FhirError::BadValue("dispenseRequest.quantity.value"))?;
    Ok(Some(q))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn order_json() -> Value {
        json!({
            "resourceType": "MedicationRequest",
            "id": "rx-1001",
            "status": "active",
            "intent": "order",
            "subject": { "reference": "Patient/MRN-42" },
            "medicationCodeableConcept": { "text": "heparin 5000 units/mL" },
            "dispenseRequest": { "quantity": { "value": 2.0 } }
        })
    }

    fn bytes(v: &Value) -> Vec<u8> {
        serde_json::to_vec(v).expect("serialises")
    }

    #[test]
    fn valid_order_is_read() {
        let o = parse_medication_request(&bytes(&order_json())).expect("valid");
        assert_eq!(o.id, "rx-1001");
        assert_eq!(o.patient_id, "MRN-42");
        assert_eq!(o.drug, "heparin 5000 units/mL");
        assert_eq!(o.quantity, Some(2.0));
        assert_eq!(o.status, "active");
    }

    #[test]
    fn coding_display_is_used_when_text_is_absent() {
        let mut v = order_json();
        v["medicationCodeableConcept"] = json!({ "coding": [{ "display": "morphine" }] });
        let o = parse_medication_request(&bytes(&v)).expect("valid");
        assert_eq!(o.drug, "morphine");
    }

    #[test]
    fn quantity_is_optional() {
        let mut v = order_json();
        v.as_object_mut().expect("object").remove("dispenseRequest");
        assert_eq!(
            parse_medication_request(&bytes(&v))
                .expect("valid")
                .quantity,
            None
        );
    }

    #[test]
    fn other_resource_types_are_refused() {
        let mut v = order_json();
        v["resourceType"] = json!("Observation");
        assert_eq!(
            parse_medication_request(&bytes(&v)),
            Err(FhirError::WrongResourceType)
        );
    }

    #[test]
    fn unknown_status_is_refused() {
        let mut v = order_json();
        v["status"] = json!("maybe");
        assert_eq!(
            parse_medication_request(&bytes(&v)),
            Err(FhirError::UnknownStatus)
        );
    }

    #[test]
    fn subject_must_reference_a_patient() {
        let mut v = order_json();
        v["subject"] = json!({ "reference": "Practitioner/7" });
        assert_eq!(
            parse_medication_request(&bytes(&v)),
            Err(FhirError::BadValue("subject.reference"))
        );
    }

    #[test]
    fn non_positive_quantity_is_refused() {
        for bad in [json!(0), json!(-1), json!("two")] {
            let mut v = order_json();
            v["dispenseRequest"]["quantity"]["value"] = bad;
            assert_eq!(
                parse_medication_request(&bytes(&v)),
                Err(FhirError::BadValue("dispenseRequest.quantity.value"))
            );
        }
    }

    #[test]
    fn oversized_resource_is_refused_before_parsing() {
        let big = vec![b' '; MAX_RESOURCE_BYTES + 1];
        assert_eq!(
            parse_medication_request(&big),
            Err(FhirError::TooLarge {
                len: MAX_RESOURCE_BYTES + 1
            })
        );
    }

    #[test]
    fn deeply_nested_json_is_refused_without_overflowing_the_stack() {
        let depth = 2_000;
        let text = format!("{}{}", "[".repeat(depth), "]".repeat(depth));
        assert_eq!(
            parse_medication_request(text.as_bytes()),
            Err(FhirError::InvalidJson)
        );
    }

    #[test]
    fn non_object_is_refused() {
        assert_eq!(
            parse_medication_request(b"[1,2]"),
            Err(FhirError::NotAnObject)
        );
    }

    #[test]
    fn round_trip_through_to_value_preserves_the_order() {
        let o = parse_medication_request(&bytes(&order_json())).expect("valid");
        let again = parse_medication_request(&bytes(&o.to_value())).expect("valid");
        assert_eq!(o, again);
    }

    #[test]
    fn debug_output_redacts_the_patient() {
        let o = parse_medication_request(&bytes(&order_json())).expect("valid");
        let shown = format!("{o:?}");
        assert!(!shown.contains("MRN-42"), "{shown}");
        assert!(shown.contains("heparin"));
    }

    #[test]
    fn mutated_json_never_panics() {
        let base = bytes(&order_json());
        let mut state = 0xFEED_F00Du64;
        for _ in 0..10_000 {
            let mut input = base.clone();
            for _ in 0..3 {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                let i = usize::try_from(state >> 33).unwrap_or(0) % input.len();
                input[i] = u8::try_from((state >> 20) % 256).unwrap_or(b'?');
            }
            let _ = parse_medication_request(&input);
        }
    }
}
