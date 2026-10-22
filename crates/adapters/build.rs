//! Export the pinned Codex CLI schema into OUT_DIR without starting a model task.

use schemars::schema::{InstanceType, Metadata, RootSchema, Schema, SchemaObject, SingleOrVec};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, VecDeque},
    ffi::OsString,
    fs,
    path::PathBuf,
    process::Command,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const ROOTS: &[&str] = &[
    "FsWriteFileParams",
    "FsWriteFileResponse",
    "FsCreateDirectoryParams",
    "FsCreateDirectoryResponse",
    "ErrorNotification",
    "RequestId",
    "ToolRequestUserInputParams",
    "ToolRequestUserInputResponse",
    "ServerRequestResolvedNotification",
    "TurnCompletedNotification",
    "InitializeParams",
    "InitializeResponse",
    "ThreadListParams",
    "ThreadListResponse",
    "ThreadReadParams",
    "ThreadReadResponse",
    "ThreadLoadedListParams",
    "ThreadLoadedListResponse",
    "ThreadTurnsListParams",
    "ThreadTurnsListResponse",
    "ThreadItemsListParams",
    "ThreadItemsListResponse",
    "ThreadStartParams",
    "ThreadStartResponse",
    "ThreadResumeParams",
    "ThreadResumeResponse",
    "TurnStartParams",
    "TurnStartResponse",
    "TurnSteerParams",
    "TurnSteerResponse",
    "ThreadSetNameParams",
    "ThreadSetNameResponse",
];

#[derive(Deserialize)]
struct Bundle {
    definitions: Definitions,
}

#[derive(Deserialize)]
struct Definitions {
    #[serde(default)]
    v1: BTreeMap<String, Schema>,
    #[serde(default)]
    v2: BTreeMap<String, Schema>,
    #[serde(flatten)]
    common: BTreeMap<String, Schema>,
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=CODEX_SCHEMA_BIN");
    println!("cargo:rerun-if-env-changed=PATH");
    if let Err(error) = generate() {
        panic!(
            "Codex schema generation failed: {error}. Install the current Codex CLI or set CODEX_SCHEMA_BIN to the intended binary; no schema snapshot is used."
        );
    }
}

fn generate() -> Result<()> {
    let binary =
        find_binary(std::env::var_os("CODEX_SCHEMA_BIN").unwrap_or_else(|| "codex".into()))?;
    println!("cargo:rerun-if-changed={}", binary.display());
    let version = Command::new(&binary).arg("--version").output()?;
    if !version.status.success() {
        return Err("Codex version query failed".into());
    }
    let version = String::from_utf8(version.stdout)?.trim().to_owned();
    if !version.starts_with("codex-cli ") || version.contains(['\n', '\r']) {
        return Err("invalid Codex version response".into());
    }
    println!("cargo:warning=Generating protocol types using {version}");

    let output = PathBuf::from(std::env::var_os("OUT_DIR").ok_or("OUT_DIR missing")?);
    let exported = output.join("codex-schema");
    if exported.exists() {
        fs::remove_dir_all(&exported)?;
    }
    // Export only. Never starts, resumes, upgrades or restarts an app-server.
    let export = Command::new(&binary)
        .args(["app-server", "generate-json-schema", "--out"])
        .arg(&exported)
        .output()?;
    if !export.status.success() {
        return Err("Codex schema export failed (CLI output redacted)".into());
    }
    let Bundle { definitions } = serde_json::from_slice(&fs::read(
        exported.join("codex_app_server_protocol.schemas.json"),
    )?)?;
    let Definitions { mut common, v1, v2 } = definitions;
    drop(v1);
    common.extend(v2);
    for name in ROOTS {
        for file in [
            exported.join(format!("{name}.json")),
            exported.join("v2").join(format!("{name}.json")),
        ] {
            if file.is_file() {
                let root: RootSchema = serde_json::from_slice(&fs::read(file)?)?;
                common.extend(root.definitions);
                common.insert((*name).into(), Schema::Object(root.schema));
                break;
            }
        }
    }
    let mut selected = BTreeMap::new();
    let mut pending: VecDeque<String> = ROOTS.iter().map(|name| (*name).into()).collect();
    while let Some(name) = pending.pop_front() {
        if selected.contains_key(&name) {
            continue;
        }
        let mut schema = common.get(&name).cloned().ok_or_else(|| {
            format!("required schema {name} is missing; the CLI is too old or incompatible")
        })?;
        normalize(&mut schema, &mut pending)?;
        selected.insert(name, schema);
    }
    let schema = RootSchema {
        meta_schema: Some("http://json-schema.org/draft-07/schema#".into()),
        schema: SchemaObject {
            instance_type: Some(SingleOrVec::Single(Box::new(InstanceType::Object))),
            metadata: Some(Box::new(Metadata {
                title: Some("CodexProtocol".into()),
                ..Default::default()
            })),
            ..Default::default()
        },
        definitions: selected,
    };
    fs::write(
        output.join("codex-protocol.json"),
        serde_json::to_vec_pretty(&schema)?,
    )?;
    fs::write(output.join("schema-version.txt"), &version)?;
    println!("cargo:rustc-env=CODEX_SCHEMA_VERSION={version}");
    println!("cargo:rustc-env=CODEX_SCHEMA_BIN_PATH={}", binary.display());
    let mut settings = typify::TypeSpaceSettings::default();
    settings.with_struct_builder(true);
    let mut types = typify::TypeSpace::new(&settings);
    let mut generator_schema = schema;
    // CodexErrorInfo is an open scalar-or-object anyOf. typify 0.5 treats
    // overlapping anyOf branches as a flattened struct, which cannot decode
    // official string errors such as "other". Ask for an ordered untagged Rust
    // union while retaining every exported branch (including open fallbacks).
    // The saved projection above retains the original anyOf validation schema.
    if let Some(Schema::Object(error)) = generator_schema.definitions.get_mut("CodexErrorInfo")
        && let Some(subschemas) = error.subschemas.as_mut()
        && let Some(branches) = subschemas.any_of.take()
    {
        subschemas.one_of = Some(branches);
    }
    types.add_root_schema(generator_schema)?;
    let syntax = syn::parse2(types.to_stream())?;
    fs::write(output.join("protocol.rs"), prettyplease::unparse(&syntax))?;
    Ok(())
}

fn find_binary(binary: OsString) -> Result<PathBuf> {
    let path = PathBuf::from(&binary);
    if path.components().count() > 1 || path.is_absolute() {
        return Ok(path.canonicalize()?);
    }
    for directory in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
        let candidate = directory.join(&binary);
        if candidate.is_file() {
            return Ok(candidate.canonicalize()?);
        }
    }
    Err("Codex executable was not found in PATH".into())
}

fn normalize(schema: &mut Schema, references: &mut VecDeque<String>) -> Result<()> {
    let Schema::Object(object) = schema else {
        return Ok(());
    };
    if let Some(reference) = &mut object.reference {
        *reference = reference.replace("#/definitions/v2/", "#/definitions/");
        let name = reference
            .strip_prefix("#/definitions/")
            .ok_or("non-local schema reference")?;
        references.push_back(name.into());
    }
    if let Some(sub) = &mut object.subschemas {
        for group in [&mut sub.all_of, &mut sub.any_of, &mut sub.one_of]
            .into_iter()
            .flatten()
        {
            for schema in group {
                normalize(schema, references)?;
            }
        }
        for schema in [
            &mut sub.not,
            &mut sub.if_schema,
            &mut sub.then_schema,
            &mut sub.else_schema,
        ]
        .into_iter()
        .flatten()
        {
            normalize(schema, references)?;
        }
    }
    if let Some(array) = &mut object.array {
        match &mut array.items {
            Some(SingleOrVec::Single(schema)) => normalize(schema, references)?,
            Some(SingleOrVec::Vec(schemas)) => {
                for schema in schemas {
                    normalize(schema, references)?;
                }
            }
            None => {}
        }
        for schema in [&mut array.additional_items, &mut array.contains]
            .into_iter()
            .flatten()
        {
            normalize(schema, references)?;
        }
    }
    if let Some(object) = &mut object.object {
        for schema in object
            .properties
            .values_mut()
            .chain(object.pattern_properties.values_mut())
        {
            normalize(schema, references)?;
        }
        for schema in [
            &mut object.additional_properties,
            &mut object.property_names,
        ]
        .into_iter()
        .flatten()
        {
            normalize(schema, references)?;
        }
    }
    Ok(())
}
