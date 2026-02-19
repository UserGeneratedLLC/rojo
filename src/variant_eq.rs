use rbx_dom_weak::types::{PhysicalProperties, Variant, Vector3};

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
            a.weight == b.weight
                && a.style == b.style
                && a.family == b.family
                && a.cached_face_id == b.cached_face_id
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
}
