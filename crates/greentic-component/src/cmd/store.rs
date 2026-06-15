use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use clap::{Args, Subcommand};
use serde_json::Value;

use crate::cmd::i18n;
use crate::path_safety::normalize_under_root;
use greentic_distributor_client::{CachePolicy, DistClient, DistOptions, ResolvePolicy};

#[derive(Subcommand, Debug, Clone)]
pub enum StoreCommand {
    /// Fetch a component from a source and write the wasm bytes to disk
    Fetch(StoreFetchArgs),
    /// Publish a component to the Greentic Store as a ComponentExtension
    Publish(crate::cmd::store_publish::StorePublishArgs),
}

#[derive(Args, Debug, Clone)]
pub struct StoreFetchArgs {
    /// Destination directory for the fetched component bytes
    #[arg(long, value_name = "DIR")]
    pub out: PathBuf,
    /// Optional cache directory for fetched components
    #[arg(long, value_name = "DIR")]
    pub cache_dir: Option<PathBuf>,
    /// Source reference to resolve (file://, oci://, repo://, store://, etc.)
    #[arg(value_name = "SOURCE")]
    pub source: String,
}

pub fn run(command: StoreCommand) -> Result<()> {
    match command {
        StoreCommand::Fetch(args) => fetch(args),
        StoreCommand::Publish(args) => crate::cmd::store_publish::run(args),
    }
}

fn fetch(args: StoreFetchArgs) -> Result<()> {
    let source = resolve_source(&args.source)?;
    let mut opts = DistOptions::default();
    if let Some(cache_dir) = &args.cache_dir {
        opts.cache_dir = cache_dir.clone();
    }
    let client = DistClient::new(opts);
    let rt =
        tokio::runtime::Runtime::new().context(i18n::tr_lit("failed to create async runtime"))?;
    let parsed = client.parse_source(&source)?;
    let resolved = rt
        .block_on(async {
            let descriptor = client.resolve(parsed, ResolvePolicy).await?;
            client.fetch(&descriptor, CachePolicy).await
        })
        .context(i18n::tr_lit("store fetch failed"))?;
    let cache_path = resolved
        .cache_path
        .ok_or_else(|| anyhow!(i18n::tr_lit("resolved source has no cached component path")))?;
    let (out_dir, wasm_override) = resolve_output_paths(&args.out)?;
    fs::create_dir_all(&out_dir).with_context(|| {
        i18n::tr_lit("failed to create output dir {}").replacen(
            "{}",
            &out_dir.display().to_string(),
            1,
        )
    })?;
    let manifest_cache_path = cache_path
        .parent()
        .map(|dir| dir.join("component.manifest.json"));
    let manifest_out_path = out_dir.join("component.manifest.json");
    let mut wasm_out_path = wasm_override
        .clone()
        .unwrap_or_else(|| out_dir.join("component.wasm"));
    if let Some(manifest_cache_path) = manifest_cache_path
        && manifest_cache_path.exists()
    {
        let manifest_bytes = fs::read(&manifest_cache_path).with_context(|| {
            i18n::tr_lit("failed to read cached manifest {}").replacen(
                "{}",
                &manifest_cache_path.display().to_string(),
                1,
            )
        })?;
        fs::write(&manifest_out_path, &manifest_bytes).with_context(|| {
            i18n::tr_lit("failed to write manifest {}").replacen(
                "{}",
                &manifest_out_path.display().to_string(),
                1,
            )
        })?;
        let manifest: Value = serde_json::from_slice(&manifest_bytes).with_context(|| {
            i18n::tr_lit("failed to parse component.manifest.json from {}").replacen(
                "{}",
                &manifest_cache_path.display().to_string(),
                1,
            )
        })?;
        if let Some(component_wasm) = manifest
            .get("artifacts")
            .and_then(|artifacts| artifacts.get("component_wasm"))
            .and_then(|value| value.as_str())
        {
            let candidate = PathBuf::from(component_wasm);
            if wasm_override.is_none() {
                wasm_out_path = normalize_under_root(&out_dir, &candidate).with_context(|| {
                    i18n::tr_lit("invalid artifacts.component_wasm path `{}`").replacen(
                        "{}",
                        component_wasm,
                        1,
                    )
                })?;
                if let Some(parent) = wasm_out_path.parent() {
                    fs::create_dir_all(parent).with_context(|| {
                        i18n::tr_lit("failed to create output dir {}").replacen(
                            "{}",
                            &parent.display().to_string(),
                            1,
                        )
                    })?;
                }
            }
        }
    }
    fs::copy(&cache_path, &wasm_out_path).with_context(|| {
        i18n::tr_lit("failed to copy cached component {} to {}")
            .replacen("{}", &cache_path.display().to_string(), 1)
            .replacen("{}", &wasm_out_path.display().to_string(), 1)
    })?;
    println!(
        "{}",
        i18n::tr_lit("Wrote {} (digest {}) for source {}")
            .replacen("{}", &wasm_out_path.display().to_string(), 1)
            .replacen("{}", &resolved.digest.to_string(), 1)
            .replacen("{}", &source, 1)
    );
    if manifest_out_path.exists() {
        println!(
            "{}",
            i18n::tr_lit("Wrote {}").replacen("{}", &manifest_out_path.display().to_string(), 1)
        );
    }
    Ok(())
}

fn resolve_source(source: &str) -> Result<String> {
    let (prefix, path_str) = if let Some(rest) = source.strip_prefix("file://") {
        ("file://", rest)
    } else {
        ("", source)
    };
    let path = Path::new(path_str);
    if !path.is_dir() {
        return Ok(source.to_string());
    }

    let manifest_path = path.join("component.manifest.json");
    if manifest_path.exists() {
        let manifest_bytes = fs::read(&manifest_path).with_context(|| {
            i18n::tr_lit("failed to read component.manifest.json at {}").replacen(
                "{}",
                &manifest_path.display().to_string(),
                1,
            )
        })?;
        let manifest: Value = serde_json::from_slice(&manifest_bytes).with_context(|| {
            i18n::tr_lit("failed to parse component.manifest.json at {}").replacen(
                "{}",
                &manifest_path.display().to_string(),
                1,
            )
        })?;
        if let Some(component_wasm) = manifest
            .get("artifacts")
            .and_then(|artifacts| artifacts.get("component_wasm"))
            .and_then(|value| value.as_str())
        {
            let wasm_path =
                normalize_under_root(path, Path::new(component_wasm)).with_context(|| {
                    i18n::tr_lit("invalid artifacts.component_wasm path `{}`").replacen(
                        "{}",
                        component_wasm,
                        1,
                    )
                })?;
            return Ok(format!("{prefix}{}", wasm_path.display()));
        }
    }

    let wasm_path = path.join("component.wasm");
    if wasm_path.exists() {
        return Ok(format!("{prefix}{}", wasm_path.display()));
    }

    Err(anyhow!(
        "{}",
        i18n::tr_lit(
            "source directory {} does not contain component.manifest.json or component.wasm"
        )
        .replacen("{}", &path.display().to_string(), 1)
    ))
}

fn resolve_output_paths(out: &std::path::Path) -> Result<(PathBuf, Option<PathBuf>)> {
    if out.exists() {
        if out.is_dir() {
            return Ok((out.to_path_buf(), None));
        }
        if let Some(parent) = out.parent() {
            return Ok((parent.to_path_buf(), Some(out.to_path_buf())));
        }
        return Ok((PathBuf::from("."), Some(out.to_path_buf())));
    }

    if out.extension().is_some() {
        let parent = out.parent().unwrap_or_else(|| std::path::Path::new("."));
        return Ok((parent.to_path_buf(), Some(out.to_path_buf())));
    }

    Ok((out.to_path_buf(), None))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn resolve_output_paths_treats_existing_directory_as_output_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (out_dir, wasm) = resolve_output_paths(dir.path()).unwrap();
        assert_eq!(out_dir, dir.path());
        assert_eq!(wasm, None);
    }

    #[test]
    fn resolve_output_paths_treats_file_like_path_as_wasm_override() {
        let out = PathBuf::from("dist/component.wasm");
        let (out_dir, wasm) = resolve_output_paths(&out).unwrap();
        assert_eq!(out_dir, PathBuf::from("dist"));
        assert_eq!(wasm, Some(out));
    }

    #[test]
    fn resolve_source_prefers_manifest_artifact_inside_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        let nested = dir.path().join("dist");
        fs::create_dir_all(&nested).expect("dist dir");
        let wasm = nested.join("component.wasm");
        fs::write(&wasm, b"wasm").expect("write wasm");
        fs::write(
            dir.path().join("component.manifest.json"),
            serde_json::to_string_pretty(&json!({
                "artifacts": { "component_wasm": "dist/component.wasm" }
            }))
            .unwrap(),
        )
        .expect("write manifest");

        let resolved = resolve_source(dir.path().to_str().unwrap()).unwrap();
        assert!(resolved.ends_with("dist/component.wasm"));
    }

    #[test]
    fn resolve_source_falls_back_to_component_wasm_in_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        let wasm = dir.path().join("component.wasm");
        fs::write(&wasm, b"wasm").expect("write wasm");

        let resolved = resolve_source(dir.path().to_str().unwrap()).unwrap();
        assert!(resolved.ends_with("component.wasm"));
    }

    #[test]
    fn resolve_source_rejects_empty_directory_without_component_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let err = resolve_source(dir.path().to_str().unwrap()).expect_err("should fail");
        assert!(
            err.to_string()
                .contains("does not contain component.manifest.json or component.wasm")
        );
    }

    #[test]
    fn resolve_source_passthroughs_non_directory_references() {
        let source = "oci://registry.example.com/component:1.0.0";
        assert_eq!(resolve_source(source).unwrap(), source);
    }

    #[test]
    fn resolve_source_preserves_file_scheme_for_directory_inputs() {
        let dir = tempfile::tempdir().expect("tempdir");
        let wasm = dir.path().join("component.wasm");
        fs::write(&wasm, b"wasm").expect("write wasm");
        let source = format!("file://{}", dir.path().display());

        let resolved = resolve_source(&source).expect("resolve file dir");

        assert!(resolved.starts_with("file://"));
        assert!(resolved.ends_with("component.wasm"));
    }

    #[test]
    fn resolve_source_rejects_manifest_artifact_that_escapes_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(
            dir.path().join("component.manifest.json"),
            serde_json::to_string_pretty(&json!({
                "artifacts": { "component_wasm": "../escape.wasm" }
            }))
            .unwrap(),
        )
        .expect("write manifest");

        let err = resolve_source(dir.path().to_str().unwrap()).expect_err("escape should fail");

        assert!(
            err.to_string()
                .contains("invalid artifacts.component_wasm path")
        );
    }

    #[test]
    fn resolve_output_paths_treats_existing_file_as_override_in_parent_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        let out = dir.path().join("downloaded.wasm");
        fs::write(&out, b"old").expect("write existing output");

        let (out_dir, wasm) = resolve_output_paths(&out).expect("resolve output paths");

        assert_eq!(out_dir, dir.path());
        assert_eq!(wasm, Some(out));
    }

    #[test]
    fn resolve_output_paths_treats_extensionless_missing_path_as_directory() {
        let out = PathBuf::from("dist/component");
        let (out_dir, wasm) = resolve_output_paths(&out).expect("resolve output paths");

        assert_eq!(out_dir, out);
        assert_eq!(wasm, None);
    }
}
