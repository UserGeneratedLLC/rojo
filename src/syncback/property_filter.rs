use std::collections::HashMap;

use rbx_dom_weak::{types::Variant, Instance, Ustr, UstrMap};
use rbx_reflection::{PropertyKind, PropertySerialization, Scriptability};

use crate::{variant_eq::variant_eq, Project};

use super::SyncbackStats;

/// Per-class cache of which properties should be skipped during syncback
/// filtering. Eliminates repeated superclass-chain walks in the reflection
/// database for serialization and scriptability checks.
pub struct PropertyFilterCache {
    sync_unscriptable: bool,
    /// ClassName -> set of property names that FAIL the static checks
    /// (DoesNotSerialize or Scriptability::None when sync_unscriptable=false).
    /// Properties in this set should be skipped.
    skip_sets: HashMap<Ustr, UstrSet>,
}

type UstrSet = std::collections::HashSet<Ustr>;

impl PropertyFilterCache {
    pub fn new(project: &Project) -> Self {
        let sync_unscriptable = project
            .syncback_rules
            .as_ref()
            .and_then(|s| s.sync_unscriptable)
            .unwrap_or(false);
        Self {
            sync_unscriptable,
            skip_sets: HashMap::new(),
        }
    }

    /// Returns the set of property names to skip for a given class.
    /// Builds and caches the set on first access.
    fn skip_set_for(&mut self, class_name: &Ustr) -> &UstrSet {
        if self.skip_sets.contains_key(class_name) {
            return &self.skip_sets[class_name];
        }
        let sync_unscriptable = self.sync_unscriptable;
        let database = rbx_reflection_database::get().unwrap();
        let class_data = database.classes.get(class_name.as_str());
        let mut skip = UstrSet::new();

        if let Some(class_data) = class_data {
            // Walk all properties known to the reflection DB for this class
            // (including inherited) and mark those that fail static checks.
            let mut current = Some(class_data);
            while let Some(data) = current {
                for (prop_name, prop_data) in &data.properties {
                    let ustr_name = Ustr::from(prop_name);
                    if skip.contains(&ustr_name) {
                        continue;
                    }
                    let should_skip_serialize = match &prop_data.kind {
                        PropertyKind::Alias { alias_for } => {
                            !should_property_serialize(class_name.as_str(), alias_for)
                        }
                        PropertyKind::Canonical { serialization } => {
                            matches!(serialization, PropertySerialization::DoesNotSerialize)
                        }
                        _ => false,
                    };
                    if should_skip_serialize {
                        skip.insert(ustr_name);
                        continue;
                    }
                    if !sync_unscriptable && matches!(prop_data.scriptability, Scriptability::None)
                    {
                        skip.insert(ustr_name);
                    }
                }
                current = data
                    .superclass
                    .as_ref()
                    .and_then(|s| database.classes.get(&**s));
            }
        }

        self.skip_sets.entry(*class_name).or_insert(skip)
    }

    /// Cached version of `filter_properties_preallocated`. Fills `allocation`
    /// with properties that pass all static and value-dependent checks.
    pub fn filter_properties<'inst>(
        &mut self,
        inst: &'inst Instance,
        allocation: &mut Vec<(Ustr, &'inst Variant)>,
        stats: Option<&SyncbackStats>,
    ) {
        let database = rbx_reflection_database::get().unwrap();
        let class_data = database.classes.get(inst.class.as_str());

        if class_data.is_none() {
            if let Some(stats) = stats {
                stats.record_unknown_class(&inst.class);
            }
        }

        let skip = self.skip_set_for(&inst.class);

        if let Some(class_data) = class_data {
            let defaults = &class_data.default_properties;
            for (name, value) in &inst.properties {
                if matches!(value, Variant::Ref(_) | Variant::UniqueId(_)) {
                    continue;
                }
                if skip.contains(name) {
                    continue;
                }
                if let Some(default) = defaults.get(name.as_str()) {
                    if !variant_eq(value, default) {
                        allocation.push((*name, value));
                    }
                } else {
                    allocation.push((*name, value));
                }
            }
        } else {
            for (name, value) in &inst.properties {
                if matches!(value, Variant::Ref(_) | Variant::UniqueId(_)) {
                    continue;
                }
                allocation.push((*name, value));
            }
        }
    }
}

/// Returns a map of properties from `inst` that are both allowed under the
/// user-provided settings, are not their default value, and serialize.
pub fn filter_properties<'inst>(
    project: &Project,
    inst: &'inst Instance,
) -> UstrMap<&'inst Variant> {
    let mut map: Vec<(Ustr, &Variant)> = Vec::with_capacity(inst.properties.len());
    filter_properties_preallocated(project, inst, &mut map);

    map.into_iter().collect()
}

/// Fills `allocation` with a list of properties from `inst` that are
/// user-provided settings, are not their default value, and serialize.
pub fn filter_properties_preallocated<'inst>(
    project: &Project,
    inst: &'inst Instance,
    allocation: &mut Vec<(Ustr, &'inst Variant)>,
) {
    filter_properties_with_stats(project, inst, allocation, None);
}

/// Fills `allocation` with a list of properties from `inst` that are
/// user-provided settings, are not their default value, and serialize.
/// Optionally tracks unknown classes and properties via the stats tracker.
pub fn filter_properties_with_stats<'inst>(
    project: &Project,
    inst: &'inst Instance,
    allocation: &mut Vec<(Ustr, &'inst Variant)>,
    stats: Option<&SyncbackStats>,
) {
    let sync_unscriptable = project
        .syncback_rules
        .as_ref()
        .and_then(|s| s.sync_unscriptable)
        .unwrap_or(false);

    let database = rbx_reflection_database::get().unwrap();
    let class_data = database.classes.get(inst.class.as_str());

    // Track unknown class if not found
    if class_data.is_none() {
        if let Some(stats) = stats {
            stats.record_unknown_class(&inst.class);
        }
    }

    let predicate = |prop_name: &Ustr, prop_value: &Variant| {
        // We don't want to serialize Ref or UniqueId properties in JSON files
        if matches!(prop_value, Variant::Ref(_) | Variant::UniqueId(_)) {
            return true;
        }
        if !should_property_serialize_with_stats(&inst.class, prop_name, stats) {
            return true;
        }
        if !sync_unscriptable {
            let mut current = class_data;
            while let Some(data) = current {
                if let Some(prop_data) = data.properties.get(prop_name.as_str()) {
                    if matches!(prop_data.scriptability, Scriptability::None) {
                        return true;
                    }
                    break;
                }
                current = data
                    .superclass
                    .as_ref()
                    .and_then(|s| database.classes.get(&**s));
            }
        }
        false
    };

    if let Some(class_data) = class_data {
        let defaults = &class_data.default_properties;
        for (name, value) in &inst.properties {
            if predicate(name, value) {
                continue;
            }
            if let Some(default) = defaults.get(name.as_str()) {
                if !variant_eq(value, default) {
                    allocation.push((*name, value));
                }
            } else {
                allocation.push((*name, value));
            }
        }
    } else {
        for (name, value) in &inst.properties {
            if predicate(name, value) {
                continue;
            }
            allocation.push((*name, value));
        }
    }
}

/// Checks if a property should serialize based on the reflection database.
/// Returns false for properties with DoesNotSerialize serialization, true otherwise.
pub fn should_property_serialize(class_name: &str, prop_name: &str) -> bool {
    should_property_serialize_with_stats(class_name, prop_name, None)
}

/// Checks if a property should serialize based on the reflection database.
/// Returns false for properties with DoesNotSerialize serialization, true otherwise.
/// Optionally tracks unknown properties via the stats tracker.
pub fn should_property_serialize_with_stats(
    class_name: &str,
    prop_name: &str,
    stats: Option<&SyncbackStats>,
) -> bool {
    let database = rbx_reflection_database::get().unwrap();
    let mut current_class_name = class_name;

    loop {
        let class_data = match database.classes.get(current_class_name) {
            Some(data) => data,
            None => {
                // Unknown class - track it if stats provided
                if let Some(stats) = stats {
                    stats.record_unknown_class(current_class_name);
                }
                return true;
            }
        };
        if let Some(data) = class_data.properties.get(prop_name) {
            log::trace!("found {class_name}.{prop_name} on {current_class_name}");
            return match &data.kind {
                // It's not really clear if this can ever happen but I want to
                // support it just in case!
                PropertyKind::Alias { alias_for } => {
                    should_property_serialize_with_stats(current_class_name, alias_for, stats)
                }
                // Migrations and aliases are happily handled for us by parsers
                // so we don't really need to handle them.
                PropertyKind::Canonical { serialization } => {
                    !matches!(serialization, PropertySerialization::DoesNotSerialize)
                }
                kind => unimplemented!("unknown property kind {kind:?}"),
            };
        } else if let Some(super_class) = class_data.superclass.as_ref() {
            current_class_name = super_class;
        } else {
            break;
        }
    }

    // Property not found in class hierarchy - track it if stats provided
    if let Some(stats) = stats {
        stats.record_unknown_property(class_name, prop_name);
    }

    true
}
