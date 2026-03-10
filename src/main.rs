use anyhow::{ensure, Context};
use clap::Parser;
use coralc::module_loader::ModuleLoader;
use coralc::Compiler;
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::{NamedTempFile, TempPath};

const WORKSPACE_ROOT: &str = env!("CARGO_MANIFEST_DIR");
const BUILD_PROFILE: &str = match option_env!("PROFILE") {
    Some(value) => value,
    None => "debug",
};

#[derive(Parser, Debug)]
#[command(author, version, about = "Coral language compiler")]
struct Args {
    /// Coral source file to compile
    input: PathBuf,

    /// Optional path to write the generated LLVM IR
    #[arg(long = "emit-ir", value_name = "FILE")]
    emit_ir: Option<PathBuf>,

    /// Emit a native executable by running llc + clang on the generated IR
    #[arg(long = "emit-binary", value_name = "FILE")]
    emit_binary: Option<PathBuf>,

    /// Run the program immediately via lli while preloading the runtime library
    #[arg(long = "jit")]
    run_jit: bool,

    /// Override the runtime shared library path (defaults to target/release/*)
    #[arg(long = "runtime-lib", value_name = "PATH")]
    runtime_lib: Option<PathBuf>,

    /// Path to the lli executable used for --jit
    #[arg(long = "lli", value_name = "PATH", default_value = "lli")]
    lli: String,

    /// Path to the llc executable used for --emit-binary
    #[arg(long = "llc", value_name = "PATH", default_value = "llc")]
    llc: String,

    /// Path to the clang executable used for --emit-binary
    #[arg(long = "clang", value_name = "PATH", default_value = "clang")]
    clang: String,

    /// Write runtime metrics JSON to the given path after --jit execution
    #[arg(long = "collect-metrics", value_name = "FILE")]
    collect_metrics: Option<PathBuf>,

    /// Optimization level (0-3). Default: 0 for --jit, 2 for --emit-binary.
    #[arg(short = 'O', value_name = "LEVEL")]
    opt_level: Option<u8>,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let mut loader = ModuleLoader::with_default_std();
    let source = loader
        .load(&args.input)
        .with_context(|| format!("failed to load {}", args.input.display()))?;

    let compiler = Compiler;
    match compiler.compile_to_ir_with_warnings(&source) {
        Ok((ir, warnings)) => {
            // CC2.2: Print any warnings collected during compilation.
            for w in &warnings {
                eprintln!("warning: {}", w);
            }
            let needs_disk_ir = args.emit_binary.is_some() || args.run_jit;
            let mut temp_ir: Option<TempPath> = None;
            let ir_path_for_tools = if needs_disk_ir {
                if let Some(path) = &args.emit_ir {
                    Some(path.clone())
                } else {
                    let mut tmp = NamedTempFile::new()
                        .context("failed to create temporary IR file")?;
                    tmp.write_all(ir.as_bytes())
                        .context("failed to write temporary IR file")?;
                    let temp_path = tmp
                        .into_temp_path();
                    let path_buf = temp_path.to_path_buf();
                    temp_ir = Some(temp_path);
                    Some(path_buf)
                }
            } else {
                None
            };

            if let Some(path) = &args.emit_ir {
                fs::write(path, ir).with_context(|| {
                    format!("failed to write {}", path.display())
                })?;
            } else {
                println!("{}", ir);
            }

            let runtime_path = if args.run_jit || args.emit_binary.is_some() {
                Some(resolve_runtime_library(args.runtime_lib.clone())?)
            } else {
                None
            };

            if let Some(runtime_lib) = runtime_path {
                if let Some(path) = &args.collect_metrics {
                    if args.run_jit {
                        if let Some(parent) = path.parent() {
                            if !parent.as_os_str().is_empty() {
                                fs::create_dir_all(parent).with_context(|| {
                                    format!(
                                        "failed to create metrics directory {}",
                                        parent.display()
                                    )
                                })?;
                            }
                        }
                    } else {
                        eprintln!(
                            "note: --collect-metrics only applies to --jit. Set CORAL_RUNTIME_METRICS manually when running emitted binaries."
                        );
                    }
                }

                if args.run_jit {
                    let ir_path = ir_path_for_tools
                        .as_ref()
                        .context("jit requested but no IR file available")?;
                    let jit_opt = args.opt_level.unwrap_or(0);
                    run_lli(
                        &args.lli,
                        &runtime_lib,
                        ir_path,
                        args.collect_metrics.as_deref(),
                        jit_opt,
                    )?;
                }
                if let Some(binary_path) = &args.emit_binary {
                    let ir_path = ir_path_for_tools
                        .as_ref()
                        .context("binary emission requested but no IR file available")?;
                    let bin_opt = args.opt_level.unwrap_or(2);
                    link_native_binary(
                        &args.llc,
                        &args.clang,
                        &runtime_lib,
                        ir_path,
                        binary_path,
                        bin_opt,
                    )?;
                }
            }

            drop(temp_ir);
        }
        Err(err) => {
            eprintln!("{}", err);
            if let Some(help) = &err.diagnostic.help {
                eprintln!("help: {}", help);
            }
            std::process::exit(1);
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BuildProfileKind {
    Debug,
    Release,
}

impl BuildProfileKind {
    fn as_dir(self) -> &'static str {
        match self {
            BuildProfileKind::Debug => "debug",
            BuildProfileKind::Release => "release",
        }
    }

    fn cargo_args(self) -> &'static [&'static str] {
        match self {
            BuildProfileKind::Debug => &["build", "-p", "runtime"],
            BuildProfileKind::Release => &["build", "-p", "runtime", "--release"],
        }
    }
}

fn resolve_runtime_library(explicit: Option<PathBuf>) -> anyhow::Result<PathBuf> {
    if let Some(path) = explicit {
        ensure!(path.exists(), "runtime library not found at {}", path.display());
        return Ok(path);
    }

    ensure_runtime_artifacts()?;
    let profile = if BUILD_PROFILE == "release" {
        BuildProfileKind::Release
    } else {
        BuildProfileKind::Debug
    };

    locate_runtime_library(profile)
        .or_else(|| locate_runtime_library(BuildProfileKind::Release))
        .or_else(|| locate_runtime_library(BuildProfileKind::Debug))
        .context("unable to locate runtime shared library. Build it manually or pass --runtime-lib")
}

fn ensure_runtime_artifacts() -> anyhow::Result<()> {
    ensure_profile_runtime(BuildProfileKind::Debug)?;
    ensure_profile_runtime(BuildProfileKind::Release)?;
    Ok(())
}

fn ensure_profile_runtime(profile: BuildProfileKind) -> anyhow::Result<()> {
    let artifact_path = runtime_library_path(profile);
    if artifact_path.exists() {
        return Ok(());
    }
    let mut cmd = Command::new("cargo");
    cmd.args(profile.cargo_args());
    cmd.current_dir(WORKSPACE_ROOT);
    let status = cmd.status().context("failed to invoke cargo for runtime build")?;
    ensure!(status.success(), "cargo failed while building runtime crate");
    ensure!(
        artifact_path.exists(),
        "runtime artifact missing at {} after build",
        artifact_path.display()
    );
    Ok(())
}

fn runtime_library_path(profile: BuildProfileKind) -> PathBuf {
    PathBuf::from(WORKSPACE_ROOT)
        .join("target")
        .join(profile.as_dir())
        .join(runtime_library_filename())
}

fn locate_runtime_library(profile: BuildProfileKind) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    candidates.push(runtime_library_path(profile));
    if let Ok(mut exe_path) = env::current_exe() {
        if exe_path.pop() {
            let exe_dir = exe_path;
            candidates.push(exe_dir.join(runtime_library_filename()));
            if let Some(parent) = exe_dir.parent() {
                candidates.push(parent.join(runtime_library_filename()));
            }
        }
    }
    candidates.into_iter().find(|path| path.exists())
}

fn run_lli(
    lli: &str,
    runtime: &Path,
    ir_path: &Path,
    metrics_path: Option<&Path>,
    opt_level: u8,
) -> anyhow::Result<()> {
    let mut cmd = Command::new(lli);
    cmd.arg("-load").arg(runtime);
    if opt_level > 0 {
        cmd.arg(format!("-O{}", opt_level.min(3)));
    }
    cmd.arg(ir_path);
    if let Some(metrics) = metrics_path {
        cmd.env("CORAL_RUNTIME_METRICS", metrics);
    }
    let status = cmd
        .status()
        .with_context(|| format!("failed to invoke {}", lli))?;
    ensure!(status.success(), "lli execution failed");
    Ok(())
}

fn link_native_binary(
    llc: &str,
    clang: &str,
    runtime_lib: &Path,
    ir_path: &Path,
    output: &Path,
    opt_level: u8,
) -> anyhow::Result<()> {
    let mut obj = NamedTempFile::new().context("failed to create temporary object file")?;
    let obj_path = obj
        .path()
        .to_path_buf();

    let opt_flag = format!("-O{}", opt_level.min(3));
    let llc_status = Command::new(llc)
        .arg(ir_path)
        .arg("-filetype=obj")
        .arg(&opt_flag)
        .arg("-o")
        .arg(&obj_path)
        .status()
        .with_context(|| format!("failed to invoke {}", llc))?;
    ensure!(llc_status.success(), "llc failed to lower IR to object file");

    let runtime_dir = runtime_lib
        .parent()
        .context("runtime library path must have a parent directory")?;

    let mut clang_cmd = Command::new(clang);
    clang_cmd
        .arg(&obj_path)
        .arg(&opt_flag)
        .arg("-L")
        .arg(runtime_dir)
        .arg("-l")
        .arg("runtime")
        .arg("-lm")
        .arg("-o")
        .arg(output);

    if cfg!(target_os = "linux") {
        clang_cmd.arg("-no-pie");
    }

    if cfg!(any(target_os = "linux", target_os = "macos")) {
        let rpath_flag = format!("-Wl,-rpath,{}", runtime_dir.display());
        clang_cmd.arg(rpath_flag);
    }

    let clang_status = clang_cmd
        .status()
        .with_context(|| format!("failed to invoke {}", clang))?;
    ensure!(clang_status.success(), "clang failed to link native binary");

    obj.flush().ok();
    Ok(())
}

fn runtime_library_filename() -> &'static str {
    if cfg!(target_os = "macos") {
        "libruntime.dylib"
    } else if cfg!(target_os = "windows") {
        "runtime.dll"
    } else {
        "libruntime.so"
    }
}
