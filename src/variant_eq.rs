use lexical_write_float::{format::STANDARD, ToLexicalWithOptions};
use rbx_dom_weak::types::{PhysicalProperties, Variant, Vector3};

use crate::json::{F32_DISK_BUF_SIZE, F32_DISK_OPTIONS, F64_DISK_BUF_SIZE, F64_DISK_OPTIONS};
use crate::resolution::cleanup_f32;

const EPSILON_F32: f32 = 0.0001;
const EPSILON_F64: f64 = 0.0001;

/// Fuzzy float equality matching Lua trueEquals: absolute OR relative epsilon.
/// NaN == NaN is true.
#[inline(always)]
fn fuzzy_eq_f32(a: f32, b: f32) -> bool {
    if a.is_nan() {
        return b.is_nan();
    }
    if b.is_nan() {
        return false;
    }
    let diff = (a - b).abs();
    let max_val = a.abs().max(b.abs()).max(1.0);
    diff < EPSILON_F32 || diff < max_val * EPSILON_F32
}

#[inline(always)]
fn fuzzy_eq_f64(a: f64, b: f64) -> bool {
    if a.is_nan() {
        return b.is_nan();
    }
    if b.is_nan() {
        return false;
    }
    let diff = (a - b).abs();
    let max_val = a.abs().max(b.abs()).max(1.0);
    diff < EPSILON_F64 || diff < max_val * EPSILON_F64
}

/// Compares two variants to determine if they're equal. This correctly takes
/// float comparisons into account.
#[inline]
pub fn variant_eq(variant_a: &Variant, variant_b: &Variant) -> bool {
    if variant_a.ty() != variant_b.ty() {
        return false;
    }

    match (variant_a, variant_b) {
        (Variant::Attributes(a), Variant::Attributes(b)) => {
            // If they're not the same size, we can just abort
            if a.len() != b.len() {
                return false;
            }

            // Since Attributes are stored with a BTreeMap, the keys are sorted
            // and we can compare each map's keys in order.
            for ((a_name, a_value), (b_name, b_value)) in a.iter().zip(b.iter()) {
                if !(a_name == b_name && variant_eq(a_value, b_value)) {
                    return false;
                }
            }

            true
        }
        (Variant::Axes(a), Variant::Axes(b)) => a == b,
        (Variant::BinaryString(a), Variant::BinaryString(b)) => a == b,
        (Variant::Bool(a), Variant::Bool(b)) => a == b,
        (Variant::BrickColor(a), Variant::BrickColor(b)) => a == b,
        (Variant::CFrame(a), Variant::CFrame(b)) => {
            vector_eq(&a.position, &b.position)
                && vector_eq(&a.orientation.x, &b.orientation.x)
                && vector_eq(&a.orientation.y, &b.orientation.y)
                && vector_eq(&a.orientation.z, &b.orientation.z)
        }
        (Variant::Color3(a), Variant::Color3(b)) => {
            fuzzy_eq_f32(a.r, b.r) && fuzzy_eq_f32(a.g, b.g) && fuzzy_eq_f32(a.b, b.b)
        }
        (Variant::Color3uint8(a), Variant::Color3uint8(b)) => a == b,
        (Variant::ColorSequence(a), Variant::ColorSequence(b)) => {
            if a.keypoints.len() != b.keypoints.len() {
                return false;
            }
            let mut a_keypoints: Vec<_> = a.keypoints.iter().collect();
            let mut b_keypoints: Vec<_> = b.keypoints.iter().collect();
            a_keypoints.sort_unstable_by(|k1, k2| k1.time.partial_cmp(&k2.time).unwrap());
            b_keypoints.sort_unstable_by(|k1, k2| k1.time.partial_cmp(&k2.time).unwrap());

            for (a_kp, b_kp) in a_keypoints.iter().zip(b_keypoints) {
                if !(fuzzy_eq_f32(a_kp.time, b_kp.time)
                    && fuzzy_eq_f32(a_kp.color.r, b_kp.color.r)
                    && fuzzy_eq_f32(a_kp.color.g, b_kp.color.g)
                    && fuzzy_eq_f32(a_kp.color.b, b_kp.color.b))
                {
                    return false;
                }
            }
            true
        }
        (Variant::Content(a), Variant::Content(b)) => a == b,
        (Variant::ContentId(a), Variant::ContentId(b)) => a == b,
        (Variant::Enum(a), Variant::Enum(b)) => a == b,
        (Variant::EnumItem(a), Variant::EnumItem(b)) => a == b,
        (Variant::Faces(a), Variant::Faces(b)) => a == b,
        (Variant::Float32(a), Variant::Float32(b)) => fuzzy_eq_f32(*a, *b),
        (Variant::Float64(a), Variant::Float64(b)) => fuzzy_eq_f64(*a, *b),
        (Variant::Font(a), Variant::Font(b)) => {
            a.weight == b.weight && a.style == b.style && a.family == b.family
        }
        (Variant::Int32(a), Variant::Int32(b)) => a == b,
        (Variant::Int64(a), Variant::Int64(b)) => a == b,
        (Variant::MaterialColors(a), Variant::MaterialColors(b)) => a.encode() == b.encode(),
        (Variant::NetAssetRef(a), Variant::NetAssetRef(b)) => a == b,
        (Variant::NumberRange(a), Variant::NumberRange(b)) => {
            fuzzy_eq_f32(a.max, b.max) && fuzzy_eq_f32(a.min, b.min)
        }
        (Variant::NumberSequence(a), Variant::NumberSequence(b)) => {
            if a.keypoints.len() != b.keypoints.len() {
                return false;
            }
            let mut a_keypoints: Vec<_> = a.keypoints.iter().collect();
            let mut b_keypoints: Vec<_> = b.keypoints.iter().collect();
            a_keypoints.sort_unstable_by(|k1, k2| k1.time.partial_cmp(&k2.time).unwrap());
            b_keypoints.sort_unstable_by(|k1, k2| k1.time.partial_cmp(&k2.time).unwrap());

            for (a_kp, b_kp) in a_keypoints.iter().zip(b_keypoints) {
                if !(fuzzy_eq_f32(a_kp.time, b_kp.time)
                    && fuzzy_eq_f32(a_kp.value, b_kp.value)
                    && fuzzy_eq_f32(a_kp.envelope, b_kp.envelope))
                {
                    return false;
                }
            }
            true
        }
        (Variant::OptionalCFrame(a), Variant::OptionalCFrame(b)) => match (a, b) {
            (Some(a), Some(b)) => {
                vector_eq(&a.position, &b.position)
                    && vector_eq(&a.orientation.x, &b.orientation.x)
                    && vector_eq(&a.orientation.y, &b.orientation.y)
                    && vector_eq(&a.orientation.z, &b.orientation.z)
            }
            (None, None) => true,
            _ => false,
        },
        (Variant::PhysicalProperties(a), Variant::PhysicalProperties(b)) => match (a, b) {
            (PhysicalProperties::Default, PhysicalProperties::Default) => true,
            (PhysicalProperties::Custom(a2), PhysicalProperties::Custom(b2)) => {
                fuzzy_eq_f32(a2.density(), b2.density())
                    && fuzzy_eq_f32(a2.elasticity(), b2.elasticity())
                    && fuzzy_eq_f32(a2.friction(), b2.friction())
                    && fuzzy_eq_f32(a2.elasticity_weight(), b2.elasticity_weight())
                    && fuzzy_eq_f32(a2.friction_weight(), b2.friction_weight())
                    && fuzzy_eq_f32(a2.acoustic_absorption(), b2.acoustic_absorption())
            }
            _ => false,
        },
        (Variant::Ray(a), Variant::Ray(b)) => {
            vector_eq(&a.direction, &b.direction) && vector_eq(&a.origin, &b.origin)
        }
        (Variant::Rect(a), Variant::Rect(b)) => {
            fuzzy_eq_f32(a.max.x, b.max.x)
                && fuzzy_eq_f32(a.max.y, b.max.y)
                && fuzzy_eq_f32(a.min.x, b.min.x)
                && fuzzy_eq_f32(a.min.y, b.min.y)
        }
        (Variant::Ref(a), Variant::Ref(b)) => a == b,
        (Variant::Region3(a), Variant::Region3(b)) => {
            vector_eq(&a.max, &b.max) && vector_eq(&a.min, &b.min)
        }
        (Variant::Region3int16(a), Variant::Region3int16(b)) => a == b,
        (Variant::SecurityCapabilities(a), Variant::SecurityCapabilities(b)) => a == b,
        (Variant::SharedString(a), Variant::SharedString(b)) => a == b,
        (Variant::Tags(a), Variant::Tags(b)) => {
            let mut a_sorted: Vec<&str> = a.iter().collect();
            let mut b_sorted: Vec<&str> = b.iter().collect();
            if a_sorted.len() == b_sorted.len() {
                a_sorted.sort_unstable();
                b_sorted.sort_unstable();
                for (a_tag, b_tag) in a_sorted.into_iter().zip(b_sorted) {
                    if a_tag != b_tag {
                        return false;
                    }
                }
                true
            } else {
                false
            }
        }
        (Variant::UDim(a), Variant::UDim(b)) => {
            fuzzy_eq_f32(a.scale, b.scale) && a.offset == b.offset
        }
        (Variant::UDim2(a), Variant::UDim2(b)) => {
            fuzzy_eq_f32(a.x.scale, b.x.scale)
                && a.x.offset == b.x.offset
                && fuzzy_eq_f32(a.y.scale, b.y.scale)
                && a.y.offset == b.y.offset
        }
        (Variant::UniqueId(a), Variant::UniqueId(b)) => a == b,
        (Variant::String(a), Variant::String(b)) => a == b,
        (Variant::Vector2(a), Variant::Vector2(b)) => {
            fuzzy_eq_f32(a.x, b.x) && fuzzy_eq_f32(a.y, b.y)
        }
        (Variant::Vector2int16(a), Variant::Vector2int16(b)) => a == b,
        (Variant::Vector3(a), Variant::Vector3(b)) => vector_eq(a, b),
        (Variant::Vector3int16(a), Variant::Vector3int16(b)) => a == b,
        (a, b) => panic!(
            "unsupport variant comparison: {:?} and {:?}",
            a.ty(),
            b.ty()
        ),
    }
}

#[inline(always)]
fn vector_eq(a: &Vector3, b: &Vector3) -> bool {
    fuzzy_eq_f32(a.x, b.x) && fuzzy_eq_f32(a.y, b.y) && fuzzy_eq_f32(a.z, b.z)
}

// ============================================================================
// Disk-representation equality: "will these two values produce identical bytes
// when written to a .meta.json5 / .model.json5 file?"
//
// Used by the matching algorithms in src/snapshot/matching.rs and
// src/syncback/matching.rs where the cost metric is file modifications.
// ============================================================================

#[inline(always)]
fn disk_eq_f32(a: f32, b: f32) -> bool {
    let a = if a == 0.0 { 0.0_f32 } else { a };
    let b = if b == 0.0 { 0.0_f32 } else { b };
    if a.to_bits() == b.to_bits() {
        return true;
    }
    let mut buf_a = [0u8; F32_DISK_BUF_SIZE];
    let mut buf_b = [0u8; F32_DISK_BUF_SIZE];
    let repr_a = a.to_lexical_with_options::<STANDARD>(&mut buf_a, F32_DISK_OPTIONS);
    let repr_b = b.to_lexical_with_options::<STANDARD>(&mut buf_b, F32_DISK_OPTIONS);
    repr_a == repr_b
}

#[inline(always)]
fn disk_eq_f32_cleaned(a: f32, b: f32) -> bool {
    disk_eq_f32(cleanup_f32(a), cleanup_f32(b))
}

#[inline(always)]
fn disk_eq_f64(a: f64, b: f64) -> bool {
    let a = if a == 0.0 { 0.0_f64 } else { a };
    let b = if b == 0.0 { 0.0_f64 } else { b };
    if a.to_bits() == b.to_bits() {
        return true;
    }
    let mut buf_a = [0u8; F64_DISK_BUF_SIZE];
    let mut buf_b = [0u8; F64_DISK_BUF_SIZE];
    let repr_a = a.to_lexical_with_options::<STANDARD>(&mut buf_a, F64_DISK_OPTIONS);
    let repr_b = b.to_lexical_with_options::<STANDARD>(&mut buf_b, F64_DISK_OPTIONS);
    repr_a == repr_b
}

#[inline(always)]
fn vector_eq_disk(a: &Vector3, b: &Vector3) -> bool {
    disk_eq_f32_cleaned(a.x, b.x) && disk_eq_f32_cleaned(a.y, b.y) && disk_eq_f32_cleaned(a.z, b.z)
}

/// Compares two variants by their on-disk representation. Two values are
/// "equal" iff serializing both through resolution.rs + json.rs produces
/// identical bytes in the output file. Used by matching algorithms where
/// cost = number of file modifications.
#[inline]
pub fn variant_eq_disk(variant_a: &Variant, variant_b: &Variant) -> bool {
    if variant_a.ty() != variant_b.ty() {
        return false;
    }

    match (variant_a, variant_b) {
        (Variant::Attributes(a), Variant::Attributes(b)) => {
            if a.len() != b.len() {
                return false;
            }
            for ((a_name, a_value), (b_name, b_value)) in a.iter().zip(b.iter()) {
                if !(a_name == b_name && variant_eq_disk(a_value, b_value)) {
                    return false;
                }
            }
            true
        }
        (Variant::Axes(a), Variant::Axes(b)) => a == b,
        (Variant::BinaryString(a), Variant::BinaryString(b)) => a == b,
        (Variant::Bool(a), Variant::Bool(b)) => a == b,
        (Variant::BrickColor(a), Variant::BrickColor(b)) => a == b,
        (Variant::CFrame(a), Variant::CFrame(b)) => {
            vector_eq_disk(&a.position, &b.position)
                && vector_eq_disk(&a.orientation.x, &b.orientation.x)
                && vector_eq_disk(&a.orientation.y, &b.orientation.y)
                && vector_eq_disk(&a.orientation.z, &b.orientation.z)
        }
        (Variant::Color3(a), Variant::Color3(b)) => {
            disk_eq_f32_cleaned(a.r, b.r)
                && disk_eq_f32_cleaned(a.g, b.g)
                && disk_eq_f32_cleaned(a.b, b.b)
        }
        (Variant::Color3uint8(a), Variant::Color3uint8(b)) => a == b,
        (Variant::ColorSequence(a), Variant::ColorSequence(b)) => {
            if a.keypoints.len() != b.keypoints.len() {
                return false;
            }
            let mut a_keypoints: Vec<_> = a.keypoints.iter().collect();
            let mut b_keypoints: Vec<_> = b.keypoints.iter().collect();
            a_keypoints.sort_unstable_by(|k1, k2| k1.time.partial_cmp(&k2.time).unwrap());
            b_keypoints.sort_unstable_by(|k1, k2| k1.time.partial_cmp(&k2.time).unwrap());

            for (a_kp, b_kp) in a_keypoints.iter().zip(b_keypoints) {
                if !(disk_eq_f32_cleaned(a_kp.time, b_kp.time)
                    && disk_eq_f32_cleaned(a_kp.color.r, b_kp.color.r)
                    && disk_eq_f32_cleaned(a_kp.color.g, b_kp.color.g)
                    && disk_eq_f32_cleaned(a_kp.color.b, b_kp.color.b))
                {
                    return false;
                }
            }
            true
        }
        (Variant::Content(a), Variant::Content(b)) => a == b,
        (Variant::ContentId(a), Variant::ContentId(b)) => a == b,
        (Variant::Enum(a), Variant::Enum(b)) => a == b,
        (Variant::EnumItem(a), Variant::EnumItem(b)) => a == b,
        (Variant::Faces(a), Variant::Faces(b)) => a == b,
        (Variant::Float32(a), Variant::Float32(b)) => disk_eq_f32(*a, *b),
        (Variant::Float64(a), Variant::Float64(b)) => disk_eq_f64(*a, *b),
        (Variant::Font(a), Variant::Font(b)) => {
            a.weight == b.weight && a.style == b.style && a.family == b.family
        }
        (Variant::Int32(a), Variant::Int32(b)) => a == b,
        (Variant::Int64(a), Variant::Int64(b)) => a == b,
        (Variant::MaterialColors(a), Variant::MaterialColors(b)) => a.encode() == b.encode(),
        (Variant::NetAssetRef(a), Variant::NetAssetRef(b)) => a == b,
        (Variant::NumberRange(a), Variant::NumberRange(b)) => {
            disk_eq_f32_cleaned(a.max, b.max) && disk_eq_f32_cleaned(a.min, b.min)
        }
        (Variant::NumberSequence(a), Variant::NumberSequence(b)) => {
            if a.keypoints.len() != b.keypoints.len() {
                return false;
            }
            let mut a_keypoints: Vec<_> = a.keypoints.iter().collect();
            let mut b_keypoints: Vec<_> = b.keypoints.iter().collect();
            a_keypoints.sort_unstable_by(|k1, k2| k1.time.partial_cmp(&k2.time).unwrap());
            b_keypoints.sort_unstable_by(|k1, k2| k1.time.partial_cmp(&k2.time).unwrap());

            for (a_kp, b_kp) in a_keypoints.iter().zip(b_keypoints) {
                if !(disk_eq_f32_cleaned(a_kp.time, b_kp.time)
                    && disk_eq_f32_cleaned(a_kp.value, b_kp.value)
                    && disk_eq_f32_cleaned(a_kp.envelope, b_kp.envelope))
                {
                    return false;
                }
            }
            true
        }
        (Variant::OptionalCFrame(a), Variant::OptionalCFrame(b)) => match (a, b) {
            (Some(a), Some(b)) => {
                vector_eq_disk(&a.position, &b.position)
                    && vector_eq_disk(&a.orientation.x, &b.orientation.x)
                    && vector_eq_disk(&a.orientation.y, &b.orientation.y)
                    && vector_eq_disk(&a.orientation.z, &b.orientation.z)
            }
            (None, None) => true,
            _ => false,
        },
        (Variant::PhysicalProperties(a), Variant::PhysicalProperties(b)) => match (a, b) {
            (PhysicalProperties::Default, PhysicalProperties::Default) => true,
            (PhysicalProperties::Custom(a2), PhysicalProperties::Custom(b2)) => {
                disk_eq_f32_cleaned(a2.density(), b2.density())
                    && disk_eq_f32_cleaned(a2.elasticity(), b2.elasticity())
                    && disk_eq_f32_cleaned(a2.friction(), b2.friction())
                    && disk_eq_f32_cleaned(a2.elasticity_weight(), b2.elasticity_weight())
                    && disk_eq_f32_cleaned(a2.friction_weight(), b2.friction_weight())
                    && disk_eq_f32_cleaned(a2.acoustic_absorption(), b2.acoustic_absorption())
            }
            _ => false,
        },
        (Variant::Ray(a), Variant::Ray(b)) => {
            vector_eq_disk(&a.direction, &b.direction) && vector_eq_disk(&a.origin, &b.origin)
        }
        (Variant::Rect(a), Variant::Rect(b)) => {
            disk_eq_f32_cleaned(a.max.x, b.max.x)
                && disk_eq_f32_cleaned(a.max.y, b.max.y)
                && disk_eq_f32_cleaned(a.min.x, b.min.x)
                && disk_eq_f32_cleaned(a.min.y, b.min.y)
        }
        (Variant::Ref(a), Variant::Ref(b)) => a == b,
        (Variant::Region3(a), Variant::Region3(b)) => {
            vector_eq_disk(&a.max, &b.max) && vector_eq_disk(&a.min, &b.min)
        }
        (Variant::Region3int16(a), Variant::Region3int16(b)) => a == b,
        (Variant::SecurityCapabilities(a), Variant::SecurityCapabilities(b)) => a == b,
        (Variant::SharedString(a), Variant::SharedString(b)) => a == b,
        (Variant::Tags(a), Variant::Tags(b)) => {
            let mut a_sorted: Vec<&str> = a.iter().collect();
            let mut b_sorted: Vec<&str> = b.iter().collect();
            if a_sorted.len() == b_sorted.len() {
                a_sorted.sort_unstable();
                b_sorted.sort_unstable();
                for (a_tag, b_tag) in a_sorted.into_iter().zip(b_sorted) {
                    if a_tag != b_tag {
                        return false;
                    }
                }
                true
            } else {
                false
            }
        }
        (Variant::UDim(a), Variant::UDim(b)) => {
            disk_eq_f32_cleaned(a.scale, b.scale) && a.offset == b.offset
        }
        (Variant::UDim2(a), Variant::UDim2(b)) => {
            disk_eq_f32_cleaned(a.x.scale, b.x.scale)
                && a.x.offset == b.x.offset
                && disk_eq_f32_cleaned(a.y.scale, b.y.scale)
                && a.y.offset == b.y.offset
        }
        (Variant::UniqueId(a), Variant::UniqueId(b)) => a == b,
        (Variant::String(a), Variant::String(b)) => a == b,
        (Variant::Vector2(a), Variant::Vector2(b)) => {
            disk_eq_f32_cleaned(a.x, b.x) && disk_eq_f32_cleaned(a.y, b.y)
        }
        (Variant::Vector2int16(a), Variant::Vector2int16(b)) => a == b,
        (Variant::Vector3(a), Variant::Vector3(b)) => vector_eq_disk(a, b),
        (Variant::Vector3int16(a), Variant::Vector3int16(b)) => a == b,
        (a, b) => panic!(
            "unsupported variant disk comparison: {:?} and {:?}",
            a.ty(),
            b.ty()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_eq_matches_lua_absolute_epsilon() {
        assert!(fuzzy_eq_f32(1.0, 1.0 + 0.000099));
        assert!(!fuzzy_eq_f32(1.0, 1.0 + 0.00011));
    }

    #[test]
    fn fuzzy_eq_matches_lua_relative_epsilon() {
        assert!(fuzzy_eq_f32(10000.0, 10000.0 + 0.9));
        assert!(!fuzzy_eq_f32(10000.0, 10000.0 + 1.1));
    }

    #[test]
    fn fuzzy_eq_nan_handling() {
        assert!(fuzzy_eq_f32(f32::NAN, f32::NAN));
        assert!(!fuzzy_eq_f32(f32::NAN, 0.0));
        assert!(!fuzzy_eq_f32(0.0, f32::NAN));
    }

    #[test]
    fn fuzzy_eq_zero_and_negative_zero() {
        assert!(fuzzy_eq_f32(0.0, -0.0));
    }

    #[test]
    fn fuzzy_eq_f64_basic() {
        assert!(fuzzy_eq_f64(1.0, 1.0 + 0.000099));
        assert!(!fuzzy_eq_f64(1.0, 1.0 + 0.00011));
        assert!(fuzzy_eq_f64(f64::NAN, f64::NAN));
    }

    #[test]
    fn variant_eq_float32_with_new_epsilon() {
        assert!(variant_eq(
            &Variant::Float32(1.0),
            &Variant::Float32(1.0 + 0.000099)
        ));
        assert!(!variant_eq(
            &Variant::Float32(1.0),
            &Variant::Float32(1.0 + 0.00011)
        ));
    }

    // ================================================================
    // disk_eq / variant_eq_disk tests
    // ================================================================

    #[test]
    fn disk_eq_f32_identical_bits() {
        assert!(disk_eq_f32(1.0, 1.0));
        assert!(disk_eq_f32(0.0, 0.0));
        assert!(disk_eq_f32(-1.5, -1.5));
    }

    #[test]
    fn disk_eq_f32_negative_zero() {
        assert!(disk_eq_f32(0.0, -0.0));
    }

    #[test]
    fn disk_eq_f32_nan() {
        assert!(disk_eq_f32(f32::NAN, f32::NAN));
        assert!(!disk_eq_f32(f32::NAN, 0.0));
        assert!(!disk_eq_f32(0.0, f32::NAN));
    }

    #[test]
    fn disk_eq_f32_infinity() {
        assert!(disk_eq_f32(f32::INFINITY, f32::INFINITY));
        assert!(disk_eq_f32(f32::NEG_INFINITY, f32::NEG_INFINITY));
        assert!(!disk_eq_f32(f32::INFINITY, f32::NEG_INFINITY));
        assert!(!disk_eq_f32(f32::INFINITY, 0.0));
    }

    #[test]
    fn disk_eq_f32_fuzzy_says_equal_but_disk_differs() {
        let a: f32 = 10000.0;
        let b: f32 = 10000.5;
        assert!(
            fuzzy_eq_f32(a, b),
            "precondition: fuzzy should say equal (diff=0.5, relative threshold=1.0)"
        );
        assert!(
            !disk_eq_f32(a, b),
            "disk representations differ: 6 sig digits gives '10000' vs '10000.5'"
        );
    }

    #[test]
    fn disk_eq_f64_basic() {
        assert!(disk_eq_f64(1.0, 1.0));
        assert!(disk_eq_f64(0.0, -0.0));
        assert!(disk_eq_f64(f64::NAN, f64::NAN));
        assert!(!disk_eq_f64(1.0, 2.0));
    }

    #[test]
    fn disk_eq_f32_cleaned_tiny_values_zeroed() {
        assert!(
            disk_eq_f32_cleaned(0.0000001, 0.0),
            "cleanup_f32 zeros 1e-7 to 0.0"
        );
        assert!(
            !disk_eq_f32(0.0000001, 0.0),
            "without cleanup, 1e-7 and 0 are different on disk"
        );
    }

    #[test]
    fn disk_eq_f32_cleaned_preserves_significant_values() {
        assert!(!disk_eq_f32_cleaned(0.5, 0.6));
        assert!(disk_eq_f32_cleaned(0.5, 0.5));
    }

    #[test]
    fn variant_eq_disk_vector3_tiny_component() {
        assert!(variant_eq_disk(
            &Variant::Vector3(Vector3::new(1.0, 2.0, 0.0000001)),
            &Variant::Vector3(Vector3::new(1.0, 2.0, 0.0)),
        ));
    }

    #[test]
    fn variant_eq_disk_vector3_different() {
        assert!(!variant_eq_disk(
            &Variant::Vector3(Vector3::new(1.0, 2.0, 3.0)),
            &Variant::Vector3(Vector3::new(1.0, 2.0, 4.0)),
        ));
    }

    #[test]
    fn variant_eq_disk_color3() {
        use rbx_dom_weak::types::Color3;
        assert!(variant_eq_disk(
            &Variant::Color3(Color3::new(1.0, 0.0, 0.0)),
            &Variant::Color3(Color3::new(1.0, 0.0, 0.0)),
        ));
        assert!(!variant_eq_disk(
            &Variant::Color3(Color3::new(1.0, 0.0, 0.0)),
            &Variant::Color3(Color3::new(0.0, 1.0, 0.0)),
        ));
    }

    #[test]
    fn variant_eq_disk_float32_standalone() {
        assert!(variant_eq_disk(
            &Variant::Float32(0.5),
            &Variant::Float32(0.5)
        ));
        assert!(!variant_eq_disk(
            &Variant::Float32(0.5),
            &Variant::Float32(0.6)
        ));
    }

    #[test]
    fn variant_eq_disk_float32_diverges_from_fuzzy() {
        let a: f32 = 10000.0;
        let b: f32 = 10000.5;
        assert!(
            variant_eq(&Variant::Float32(a), &Variant::Float32(b)),
            "fuzzy says equal (diff=0.5, relative threshold=1.0)"
        );
        assert!(
            !variant_eq_disk(&Variant::Float32(a), &Variant::Float32(b)),
            "disk says different ('10000' vs '10000.5')"
        );
    }

    #[test]
    fn variant_eq_disk_non_float_types() {
        assert!(variant_eq_disk(&Variant::Bool(true), &Variant::Bool(true)));
        assert!(!variant_eq_disk(
            &Variant::Bool(true),
            &Variant::Bool(false)
        ));
        assert!(variant_eq_disk(&Variant::Int32(42), &Variant::Int32(42)));
        assert!(!variant_eq_disk(&Variant::Int32(42), &Variant::Int32(43)));
        assert!(variant_eq_disk(
            &Variant::String("hello".into()),
            &Variant::String("hello".into()),
        ));
        assert!(!variant_eq_disk(
            &Variant::String("hello".into()),
            &Variant::String("world".into()),
        ));
    }

    #[test]
    fn variant_eq_disk_udim2_scale_cleaned() {
        use rbx_dom_weak::types::{UDim, UDim2};
        assert!(variant_eq_disk(
            &Variant::UDim2(UDim2::new(UDim::new(0.5, 0), UDim::new(0.0000001, 100))),
            &Variant::UDim2(UDim2::new(UDim::new(0.5, 0), UDim::new(0.0, 100))),
        ));
    }

    #[test]
    fn variant_eq_disk_number_range() {
        use rbx_dom_weak::types::NumberRange;
        assert!(variant_eq_disk(
            &Variant::NumberRange(NumberRange::new(0.0, 1.0)),
            &Variant::NumberRange(NumberRange::new(0.0, 1.0)),
        ));
        assert!(!variant_eq_disk(
            &Variant::NumberRange(NumberRange::new(0.0, 1.0)),
            &Variant::NumberRange(NumberRange::new(0.0, 2.0)),
        ));
    }

    #[test]
    fn variant_eq_disk_tags_sorted() {
        use rbx_dom_weak::types::Tags;
        let mut tags_ab = Tags::new();
        tags_ab.push("A");
        tags_ab.push("B");
        let mut tags_ba = Tags::new();
        tags_ba.push("B");
        tags_ba.push("A");
        assert!(variant_eq_disk(
            &Variant::Tags(tags_ab),
            &Variant::Tags(tags_ba)
        ));
    }

    #[test]
    fn variant_eq_disk_attributes_recursive() {
        use rbx_dom_weak::types::Attributes;
        let mut a = Attributes::new();
        a.insert("Health".into(), Variant::Float32(100.0));
        let mut b = Attributes::new();
        b.insert("Health".into(), Variant::Float32(100.0));
        assert!(variant_eq_disk(
            &Variant::Attributes(a),
            &Variant::Attributes(b),
        ));
    }

    #[test]
    fn variant_eq_disk_cframe() {
        use rbx_dom_weak::types::{CFrame, Matrix3};
        let identity = Matrix3::new(
            Vector3::new(1.0, 0.0, 0.0),
            Vector3::new(0.0, 1.0, 0.0),
            Vector3::new(0.0, 0.0, 1.0),
        );
        assert!(variant_eq_disk(
            &Variant::CFrame(CFrame::new(Vector3::new(1.0, 2.0, 3.0), identity)),
            &Variant::CFrame(CFrame::new(Vector3::new(1.0, 2.0, 3.0), identity)),
        ));
        assert!(!variant_eq_disk(
            &Variant::CFrame(CFrame::new(Vector3::new(1.0, 2.0, 3.0), identity)),
            &Variant::CFrame(CFrame::new(Vector3::new(1.0, 2.0, 4.0), identity)),
        ));
    }

    #[test]
    fn variant_eq_disk_rect() {
        use rbx_dom_weak::types::{Rect, Vector2};
        assert!(variant_eq_disk(
            &Variant::Rect(Rect::new(
                Vector2::new(0.0, 0.0),
                Vector2::new(100.0, 100.0)
            )),
            &Variant::Rect(Rect::new(
                Vector2::new(0.0, 0.0),
                Vector2::new(100.0, 100.0)
            )),
        ));
    }

    #[test]
    fn variant_eq_disk_vector2() {
        use rbx_dom_weak::types::Vector2;
        assert!(variant_eq_disk(
            &Variant::Vector2(Vector2::new(1.0, 2.0)),
            &Variant::Vector2(Vector2::new(1.0, 2.0)),
        ));
        assert!(!variant_eq_disk(
            &Variant::Vector2(Vector2::new(1.0, 2.0)),
            &Variant::Vector2(Vector2::new(1.0, 3.0)),
        ));
    }
}
