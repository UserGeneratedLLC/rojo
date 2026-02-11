use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

use anyhow::Context;
use memofs::Vfs;
use rbx_dom_weak::{types::Variant, ustr};
use serde::{Deserialize, Serialize};

use crate::{
    snapshot::{InstanceContext, InstanceMetadata, InstanceSnapshot},
    syncback::{name_needs_slugify, slugify_name, FsSnapshot, SyncbackReturn, SyncbackSnapshot},
};

use super::{
    dir::{snapshot_dir_no_meta, syncback_dir_no_meta},
    meta_file::{AdjacentMetadata, DirectoryMetadata},
    PathExt as _,
};

pub fn snapshot_csv(
    _context: &InstanceContext,
    vfs: &Vfs,
    path: &Path,
    name: &str,
) -> anyhow::Result<Option<InstanceSnapshot>> {
    let contents = vfs.read(path)?;

    let table_contents = convert_localization_csv(&contents).with_context(|| {
        format!(
            "File was not a valid LocalizationTable CSV file: {}",
            path.display()
        )
    })?;

    let mut snapshot = InstanceSnapshot::new()
        .name(name)
        .class_name("LocalizationTable")
        .property(ustr("Contents"), table_contents)
        .metadata(
            InstanceMetadata::new()
                .instigating_source(path)
                .relevant_paths(vec![vfs.canonicalize(path)?]),
        );

    AdjacentMetadata::read_and_apply_all(vfs, path, name, &mut snapshot)?;

    Ok(Some(snapshot))
}

/// Attempts to snapshot an 'init' csv contained inside of a folder with
/// the given name.
///
/// csv named `init.csv`
/// their parents, which acts similarly to `__init__.py` from the Python world.
pub fn snapshot_csv_init(
    context: &InstanceContext,
    vfs: &Vfs,
    init_path: &Path,
    name: &str,
) -> anyhow::Result<Option<InstanceSnapshot>> {
    let folder_path = init_path.parent().unwrap();
    let dir_snapshot = snapshot_dir_no_meta(context, vfs, folder_path, name)?.unwrap();

    if dir_snapshot.class_name != "Folder" {
        anyhow::bail!(
            "init.csv can only be used if the instance produced by \
             the containing directory would be a Folder.\n\
             \n\
             The directory {} turned into an instance of class {}.",
            folder_path.display(),
            dir_snapshot.class_name
        );
    }

    let mut init_snapshot = snapshot_csv(context, vfs, init_path, &dir_snapshot.name)?.unwrap();

    // Preserve the init script's instigating_source (the actual file path)
    // before copying the directory's metadata (which has the folder path)
    let script_instigating_source = init_snapshot.metadata.instigating_source.take();

    init_snapshot.children = dir_snapshot.children;
    init_snapshot.metadata = dir_snapshot.metadata;

    // Restore the init script's instigating_source so two-way sync writes
    // to the actual file (e.g., init.csv) instead of the directory
    init_snapshot.metadata.instigating_source = script_instigating_source;

    DirectoryMetadata::read_and_apply_all(vfs, folder_path, &mut init_snapshot)?;

    Ok(Some(init_snapshot))
}

pub fn syncback_csv<'sync>(
    snapshot: &SyncbackSnapshot<'sync>,
) -> anyhow::Result<SyncbackReturn<'sync>> {
    let new_inst = snapshot.new_inst();

    let contents =
        if let Some(Variant::String(content)) = new_inst.properties.get(&ustr("Contents")) {
            content.as_str()
        } else {
            anyhow::bail!("LocalizationTables must have a `Contents` property that is a String")
        };
    let mut fs_snapshot = FsSnapshot::new();
    fs_snapshot.add_file(&snapshot.path, localization_to_csv(contents)?);

    let meta = AdjacentMetadata::from_syncback_snapshot(snapshot, snapshot.path.clone())?;
    if let Some(mut meta) = meta {
        // LocalizationTables have relatively few properties that we care
        // about, so shifting is fine.
        meta.properties.shift_remove(&ustr("Contents"));

        if !meta.is_empty() {
            let parent = snapshot.path.parent_err()?;
            let meta_name = snapshot
                .path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("");
            let meta_name = if meta_name.is_empty() {
                let instance_name = &new_inst.name;
                if name_needs_slugify(instance_name) {
                    slugify_name(instance_name)
                } else {
                    instance_name.clone()
                }
            } else {
                meta_name.to_string()
            };
            fs_snapshot.add_file(
                parent.join(format!("{}.meta.json5", meta_name)),
                crate::json::to_vec_pretty_sorted(&meta).context("cannot serialize metadata")?,
            )
        }
    }

    Ok(SyncbackReturn {
        fs_snapshot,
        children: Vec::new(),
        removed_children: Vec::new(),
    })
}

pub fn syncback_csv_init<'sync>(
    snapshot: &SyncbackSnapshot<'sync>,
) -> anyhow::Result<SyncbackReturn<'sync>> {
    let new_inst = snapshot.new_inst();

    let contents =
        if let Some(Variant::String(content)) = new_inst.properties.get(&ustr("Contents")) {
            content.as_str()
        } else {
            anyhow::bail!("LocalizationTables must have a `Contents` property that is a String")
        };

    let mut dir_syncback = syncback_dir_no_meta(snapshot)?;
    dir_syncback.fs_snapshot.add_file(
        snapshot.path.join("init.csv"),
        localization_to_csv(contents)?,
    );

    let meta = DirectoryMetadata::from_syncback_snapshot(snapshot, snapshot.path.clone())?;
    if let Some(mut meta) = meta {
        // LocalizationTables have relatively few properties that we care
        // about, so shifting is fine.
        meta.properties.shift_remove(&ustr("Contents"));
        if !meta.is_empty() {
            dir_syncback.fs_snapshot.add_file(
                snapshot.path.join("init.meta.json5"),
                crate::json::to_vec_pretty_sorted(&meta)
                    .context("could not serialize new init.meta.json5")?,
            );
        }
    }

    Ok(dir_syncback)
}

/// Struct that holds any valid row from a Roblox CSV translation table.
///
/// We manually deserialize into this table from CSV, but let serde_json handle
/// serialization.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LocalizationEntry<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    key: Option<Cow<'a, str>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    context: Option<Cow<'a, str>>,

    // Roblox writes `examples` for LocalizationTable's Content property, which
    // causes it to not roundtrip correctly.
    // This is reported here: https://devforum.roblox.com/t/2908720.
    //
    // To support their mistake, we support an alias named `examples`.
    #[serde(skip_serializing_if = "Option::is_none", alias = "examples")]
    example: Option<Cow<'a, str>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<Cow<'a, str>>,

    // We use a BTreeMap here to get deterministic output order.
    values: BTreeMap<Cow<'a, str>, Cow<'a, str>>,
}

/// Normally, we'd be able to let the csv crate construct our struct for us.
///
/// However, because of a limitation with Serde's 'flatten' feature, it's not
/// possible presently to losslessly collect extra string values while using
/// csv+Serde.
///
/// https://github.com/BurntSushi/rust-csv/issues/151
///
/// This function operates in one step in order to minimize data-copying.
fn convert_localization_csv(contents: &[u8]) -> Result<String, csv::Error> {
    let mut reader = csv::Reader::from_reader(contents);

    let headers = reader.headers()?.clone();

    let mut records = Vec::new();

    for record in reader.into_records() {
        records.push(record?);
    }

    let mut entries = Vec::new();

    for record in &records {
        let mut entry = LocalizationEntry::default();

        for (header, value) in headers.iter().zip(record.into_iter()) {
            if header.is_empty() || value.is_empty() {
                continue;
            }

            match header {
                "Key" => entry.key = Some(Cow::Borrowed(value)),
                "Source" => entry.source = Some(Cow::Borrowed(value)),
                "Context" => entry.context = Some(Cow::Borrowed(value)),
                "Example" => entry.example = Some(Cow::Borrowed(value)),
                _ => {
                    entry
                        .values
                        .insert(Cow::Borrowed(header), Cow::Borrowed(value));
                }
            }
        }

        if entry.key.is_none() && entry.source.is_none() {
            continue;
        }

        entries.push(entry);
    }

    let encoded =
        json5::to_string(&entries).expect("Could not encode JSON5 for localization table");

    Ok(encoded)
}

/// Takes a localization table (as a string) and converts it into a CSV file.
///
/// The CSV file is ordered, so it should be deterministic.
fn localization_to_csv(csv_contents: &str) -> anyhow::Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut writer = csv::Writer::from_writer(&mut out);

    let mut csv: Vec<LocalizationEntry> =
        json5::from_str(csv_contents).context("cannot decode JSON from localization table")?;

    // TODO sort this better
    csv.sort_by(|a, b| a.source.partial_cmp(&b.source).unwrap());

    let mut headers = vec!["Key", "Source", "Context", "Example"];
    // We want both order and a lack of duplicates, so we use a BTreeSet.
    let mut extra_headers = BTreeSet::new();
    for entry in &csv {
        for lang in entry.values.keys() {
            extra_headers.insert(lang.as_ref());
        }
    }
    headers.extend(extra_headers.iter());

    writer
        .write_record(&headers)
        .context("could not write headers for localization table")?;

    let mut record: Vec<&str> = Vec::with_capacity(headers.len());
    for entry in &csv {
        record.push(entry.key.as_deref().unwrap_or_default());
        record.push(entry.source.as_deref().unwrap_or_default());
        record.push(entry.context.as_deref().unwrap_or_default());
        record.push(entry.example.as_deref().unwrap_or_default());

        let values = &entry.values;
        for header in &extra_headers {
            record.push(values.get(*header).map(AsRef::as_ref).unwrap_or_default());
        }

        writer
            .write_record(&record)
            .context("cannot write record for localization table")?;
        record.clear();
    }

    // We must drop `writer` here to regain access to `out`.
    drop(writer);

    Ok(out)
}

#[cfg(test)]
mod test {
    use super::*;

    use memofs::{InMemoryFs, VfsSnapshot};

    #[test]
    fn csv_from_vfs() {
        let mut imfs = InMemoryFs::new();
        imfs.load_snapshot(
            "/foo.csv",
            VfsSnapshot::file(
                r#"
Key,Source,Context,Example,es
Ack,Ack!,,An exclamation of despair,¡Ay!"#,
            ),
        )
        .unwrap();

        let vfs = Vfs::new(imfs);

        let instance_snapshot = snapshot_csv(
            &InstanceContext::default(),
            &vfs,
            Path::new("/foo.csv"),
            "foo",
        )
        .unwrap()
        .unwrap();

        insta::assert_yaml_snapshot!(instance_snapshot);
    }

    #[test]
    fn csv_with_meta() {
        let mut imfs = InMemoryFs::new();
        imfs.load_snapshot(
            "/foo.csv",
            VfsSnapshot::file(
                r#"
Key,Source,Context,Example,es
Ack,Ack!,,An exclamation of despair,¡Ay!"#,
            ),
        )
        .unwrap();
        imfs.load_snapshot(
            "/foo.meta.json5",
            VfsSnapshot::file(r#"{ "ignoreUnknownInstances": true }"#),
        )
        .unwrap();

        let vfs = Vfs::new(imfs);

        let instance_snapshot = snapshot_csv(
            &InstanceContext::default(),
            &vfs,
            Path::new("/foo.csv"),
            "foo",
        )
        .unwrap()
        .unwrap();

        insta::assert_yaml_snapshot!(instance_snapshot);
    }

    #[test]
    fn csv_init() {
        let mut imfs = InMemoryFs::new();
        imfs.load_snapshot(
            "/root",
            VfsSnapshot::dir([(
                "init.csv",
                VfsSnapshot::file(
                    r#"
Key,Source,Context,Example,es
Ack,Ack!,,An exclamation of despair,¡Ay!"#,
                ),
            )]),
        )
        .unwrap();

        let vfs = Vfs::new(imfs);

        let instance_snapshot = snapshot_csv_init(
            &InstanceContext::new(),
            &vfs,
            Path::new("/root/init.csv"),
            "root",
        )
        .unwrap()
        .unwrap();

        insta::with_settings!({ sort_maps => true }, {
            insta::assert_yaml_snapshot!(instance_snapshot);
        });
    }

    #[test]
    fn csv_init_with_meta() {
        let mut imfs = InMemoryFs::new();
        imfs.load_snapshot(
            "/root",
            VfsSnapshot::dir([
                (
                    "init.csv",
                    VfsSnapshot::file(
                        r#"
Key,Source,Context,Example,es
Ack,Ack!,,An exclamation of despair,¡Ay!"#,
                    ),
                ),
                (
                    "init.meta.json5",
                    VfsSnapshot::file(r#"{"id": "manually specified"}"#),
                ),
            ]),
        )
        .unwrap();

        let vfs = Vfs::new(imfs);

        let instance_snapshot = snapshot_csv_init(
            &InstanceContext::new(),
            &vfs,
            Path::new("/root/init.csv"),
            "root",
        )
        .unwrap()
        .unwrap();

        insta::with_settings!({ sort_maps => true }, {
            insta::assert_yaml_snapshot!(instance_snapshot);
        });
    }
}
