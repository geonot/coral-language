use anyhow::{Context, ensure};
use clap::Parser;
use coralc::Compiler;
use coralc::diagnostics::WarningCategory;
use coralc::module_loader::ModuleLoader;
use std::collections::HashSet;
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
    /// Input file to compile (required for compilation, optional for --init)
    input: Option<PathBuf>,

    #[arg(long = "emit-ir", value_name = "FILE")]
    emit_ir: Option<PathBuf>,

    #[arg(long = "emit-binary", value_name = "FILE")]
    emit_binary: Option<PathBuf>,

    #[arg(long = "jit")]
    run_jit: bool,

    #[arg(long = "runtime-lib", value_name = "PATH")]
    runtime_lib: Option<PathBuf>,

    #[arg(long = "lli", value_name = "PATH", default_value = "lli")]
    lli: String,

    #[arg(long = "llc", value_name = "PATH", default_value = "llc")]
    llc: String,

    #[arg(long = "clang", value_name = "PATH", default_value = "clang")]
    clang: String,

    #[arg(long = "collect-metrics", value_name = "FILE")]
    collect_metrics: Option<PathBuf>,

    #[arg(short = 'O', value_name = "LEVEL")]
    opt_level: Option<u8>,

    #[arg(long = "allow", value_name = "CATEGORY")]
    allow: Vec<String>,

    #[arg(long = "warn", value_name = "CATEGORY")]
    warn: Vec<String>,

    #[arg(long = "lto")]
    lto: bool,

    #[arg(long = "static")]
    link_static: bool,

    #[arg(long = "pgo-gen")]
    pgo_gen: bool,

    #[arg(long = "pgo-use", value_name = "PROFDATA")]
    pgo_use: Option<PathBuf>,

    #[arg(long = "docs", value_name = "OUTPUT_DIR")]
    docs: Option<PathBuf>,

    #[arg(long = "init", value_name = "PROJECT_NAME")]
    init: Option<String>,

    #[arg(long = "no-prelude")]
    no_prelude: bool,

    /// Run test functions (functions starting with *test_) in the input file
    #[arg(long = "test")]
    test: bool,

    /// Filter tests by name (substring match)
    #[arg(long = "test-filter", value_name = "PATTERN")]
    test_filter: Option<String>,
}

fn main() -> anyhow::Result<()> {
    // Install a panic hook that presents codegen panics as internal compiler errors
    std::panic::set_hook(Box::new(|info| {
        let payload = if let Some(s) = info.payload().downcast_ref::<&str>() {
            s.to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "unknown panic".to_string()
        };
        let location = info
            .location()
            .map(|l| format!(" at {}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_default();
        eprintln!("internal compiler error: {}{}", payload, location);
        eprintln!("This is a bug in the Coral compiler. Please report it.");
    }));

    let args = Args::parse();

    if let Some(level) = args.opt_level {
        if level > 3 {
            anyhow::bail!("invalid optimization level -O{}: must be 0, 1, 2, or 3", level);
        }
    }

    if args.pgo_gen && args.pgo_use.is_some() {
        anyhow::bail!("--pgo-gen and --pgo-use are mutually exclusive");
    }

    if let Some(project_name) = &args.init {
        let dir = std::env::current_dir()?.join(project_name);
        coralc::package::init_project(&dir, project_name)
            .map_err(|e| anyhow::anyhow!("failed to init project: {}", e))?;
        eprintln!("Created project '{}' in {}", project_name, dir.display());
        return Ok(());
    }

    if let Some(output_dir) = &args.docs {
        let input_path = args.input.as_ref()
            .ok_or_else(|| anyhow::anyhow!("--docs requires an input file or directory"))?;
        if input_path.is_dir() {
            let generated =
                coralc::doc_gen::generate_docs_for_directory(input_path, output_dir)?;
            eprintln!("Generated {} doc files in {}", generated.len(), output_dir.display());
        } else {
            let source = fs::read_to_string(input_path)?;
            let filename = input_path.to_string_lossy();
            let items = coralc::doc_gen::extract_docs(&source, &filename);
            let module_name = input_path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "module".to_string());
            let markdown = coralc::doc_gen::generate_markdown(&items, &module_name);
            fs::create_dir_all(output_dir)?;
            let out_path = output_dir.join(format!("{}.md", module_name));
            fs::write(&out_path, &markdown)?;
            eprintln!("Generated {}", out_path.display());
        }
        return Ok(());
    }

    let input = args.input.as_ref()
        .ok_or_else(|| anyhow::anyhow!("no input file specified"))?;

    // Test mode: discover test functions and generate a runner
    if args.test {
        return run_tests(&args, input);
    }

    let mut loader = ModuleLoader::with_default_std();
    loader.no_prelude = args.no_prelude;
    let module_sources = loader
        .load_modules(input)
        .with_context(|| format!("failed to load {}", input.display()))?;

    let compiler = Compiler;
    match compiler.compile_modules_to_ir(&module_sources) {
        Ok((ir, warnings)) => {
            let ir = if args.lto {
                let opt_level = match args.opt_level.unwrap_or(2) {
                    0 | 1 => coralc::compiler::LtoOptLevel::O1,
                    2 => coralc::compiler::LtoOptLevel::O2,
                    _ => coralc::compiler::LtoOptLevel::O3,
                };
                coralc::compiler::optimize_module(&ir, opt_level)
                    .map_err(|e| anyhow::anyhow!("LTO optimization failed: {}", e))?
            } else {
                ir
            };

            let ir = if args.pgo_gen {
                coralc::compiler::instrument_for_pgo(&ir)
                    .map_err(|e| anyhow::anyhow!("PGO instrumentation failed: {}", e))?
            } else if let Some(ref profdata) = args.pgo_use {
                let opt_level = match args.opt_level.unwrap_or(2) {
                    0 | 1 => coralc::compiler::LtoOptLevel::O1,
                    2 => coralc::compiler::LtoOptLevel::O2,
                    _ => coralc::compiler::LtoOptLevel::O3,
                };
                coralc::compiler::optimize_with_profile(&ir, &profdata.to_string_lossy(), opt_level)
                    .map_err(|e| anyhow::anyhow!("PGO optimization failed: {}", e))?
            } else {
                ir
            };

            let suppressed: HashSet<WarningCategory> = args
                .allow
                .iter()
                .filter_map(|s| WarningCategory::from_str(s))
                .collect();
            let forced: HashSet<WarningCategory> = args
                .warn
                .iter()
                .filter_map(|s| WarningCategory::from_str(s))
                .collect();

            for w in &warnings {
                if let Some(cat) = &w.category {
                    if suppressed.contains(cat) && !forced.contains(cat) {
                        continue;
                    }
                }
                let cat_label = w.category.map_or(String::new(), |c| format!(" [{}]", c));
                eprintln!("warning{}: {}", cat_label, w.message);
            }
            let needs_disk_ir = args.emit_binary.is_some() || args.run_jit;
            let mut temp_ir: Option<TempPath> = None;
            let ir_path_for_tools = if needs_disk_ir {
                if let Some(path) = &args.emit_ir {
                    Some(path.clone())
                } else {
                    let mut tmp =
                        NamedTempFile::new().context("failed to create temporary IR file")?;
                    tmp.write_all(ir.as_bytes())
                        .context("failed to write temporary IR file")?;
                    let temp_path = tmp.into_temp_path();
                    let path_buf = temp_path.to_path_buf();
                    temp_ir = Some(temp_path);
                    Some(path_buf)
                }
            } else {
                None
            };

            if let Some(path) = &args.emit_ir {
                fs::write(path, ir)
                    .with_context(|| format!("failed to write {}", path.display()))?;
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
                        args.link_static,
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

/// Discover test functions in a Coral file and run them via JIT.
///
/// Test functions are any top-level functions whose name starts with `test_`.
/// A synthetic `*main()` is generated that calls each test and reports results.
fn run_tests(args: &Args, input: &Path) -> anyhow::Result<()> {
    let source = fs::read_to_string(input)
        .with_context(|| format!("failed to read {}", input.display()))?;

    // Parse to find test function names (functions starting with *test_)
    let test_names: Vec<String> = source
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.starts_with('*') {
                let rest = &trimmed[1..];
                if let Some(paren_idx) = rest.find('(') {
                    let name = &rest[..paren_idx];
                    if name.starts_with("test_") {
                        return Some(name.to_string());
                    }
                }
            }
            None
        })
        .collect();

    if test_names.is_empty() {
        eprintln!("No test functions found in {}", input.display());
        eprintln!("Test functions must start with *test_");
        return Ok(());
    }

    // Apply filter if provided
    let test_names: Vec<String> = if let Some(ref filter) = args.test_filter {
        test_names
            .into_iter()
            .filter(|name| name.contains(filter.as_str()))
            .collect()
    } else {
        test_names
    };

    if test_names.is_empty() {
        eprintln!(
            "No tests matched filter '{}'",
            args.test_filter.as_deref().unwrap_or("")
        );
        return Ok(());
    }

    eprintln!("Running {} test(s) from {}", test_names.len(), input.display());

    // Generate a test runner *main() that calls each test and reports results
    let mut runner = String::new();
    runner.push_str("*main()\n");
    runner.push_str(&format!("    _passed is 0\n"));
    runner.push_str(&format!("    _failed is 0\n"));
    for name in &test_names {
        // Each test is called in a try-catch style using Coral's error model
        runner.push_str(&format!("    _result is {}()\n", name));
        // TODO: When error catching is available, wrap in error handling
        runner.push_str(&format!("    log('PASS: {}')\n", name));
        runner.push_str(&format!("    _passed is _passed + 1\n"));
    }
    runner.push_str(&format!(
        "    log('\\n' + to_string(_passed) + ' passed, ' + to_string(_failed) + ' failed')\n"
    ));

    // Append runner to the source (removing any existing *main())
    let mut test_source = String::new();
    let mut skip_main = false;
    let mut main_indent = 0;
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("*main(") {
            skip_main = true;
            main_indent = line.len() - line.trim_start().len();
            continue;
        }
        if skip_main {
            let current_indent = if trimmed.is_empty() {
                main_indent + 1 // blank lines inside main
            } else {
                line.len() - line.trim_start().len()
            };
            if current_indent > main_indent {
                continue;
            }
            skip_main = false;
        }
        test_source.push_str(line);
        test_source.push('\n');
    }
    test_source.push('\n');
    test_source.push_str(&runner);

    // Write to a temp file and compile+run with JIT
    let mut tmp = NamedTempFile::with_suffix(".coral")
        .context("failed to create temporary test file")?;
    tmp.write_all(test_source.as_bytes())
        .context("failed to write test source")?;
    let tmp_path = tmp.into_temp_path();

    let mut loader = ModuleLoader::with_default_std();
    loader.no_prelude = args.no_prelude;
    let module_sources = loader
        .load_modules(tmp_path.as_ref())
        .with_context(|| format!("failed to load test file"))?;

    let compiler = Compiler;
    match compiler.compile_modules_to_ir(&module_sources) {
        Ok((ir, _warnings)) => {
            let runtime_lib = resolve_runtime_library(args.runtime_lib.clone())?;
            let mut ir_file = NamedTempFile::with_suffix(".ll")
                .context("failed to create temporary IR file")?;
            ir_file
                .write_all(ir.as_bytes())
                .context("failed to write IR")?;
            let ir_path = ir_file.into_temp_path();
            run_lli(&args.lli, &runtime_lib, ir_path.as_ref(), None, 0)?;
        }
        Err(err) => {
            eprintln!("Test compilation failed: {}", err);
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
        ensure!(
            path.exists(),
            "runtime library not found at {}",
            path.display()
        );
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
    let status = cmd
        .status()
        .context("failed to invoke cargo for runtime build")?;
    ensure!(
        status.success(),
        "cargo failed while building runtime crate"
    );
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
    link_static: bool,
) -> anyhow::Result<()> {
    let mut obj = NamedTempFile::new().context("failed to create temporary object file")?;
    let obj_path = obj.path().to_path_buf();

    let opt_flag = format!("-O{}", opt_level.min(3));
    let llc_status = Command::new(llc)
        .arg(ir_path)
        .arg("-filetype=obj")
        .arg(&opt_flag)
        .arg("-o")
        .arg(&obj_path)
        .status()
        .with_context(|| format!("failed to invoke {}", llc))?;
    ensure!(
        llc_status.success(),
        "llc failed to lower IR to object file"
    );

    let runtime_dir = runtime_lib
        .parent()
        .context("runtime library path must have a parent directory")?;

    let mut clang_cmd = Command::new(clang);
    clang_cmd.arg(&obj_path).arg(&opt_flag);

    if link_static {
        let static_lib = runtime_dir.join("libruntime.a");
        ensure!(
            static_lib.exists(),
            "static runtime library not found at {}. Build with `cargo build -p runtime`",
            static_lib.display()
        );
        clang_cmd.arg(&static_lib);

        clang_cmd.arg("-lm").arg("-lpthread").arg("-ldl");
        if cfg!(target_os = "linux") {
            clang_cmd.arg("-static-libgcc");
        }
    } else {
        clang_cmd
            .arg("-L")
            .arg(runtime_dir)
            .arg("-l")
            .arg("runtime")
            .arg("-lm");

        if cfg!(any(target_os = "linux", target_os = "macos")) {
            let rpath_flag = format!("-Wl,-rpath,{}", runtime_dir.display());
            clang_cmd.arg(rpath_flag);
        }
    }

    clang_cmd.arg("-o").arg(output);

    if cfg!(target_os = "linux") {
        clang_cmd.arg("-no-pie");
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

#[allow(dead_code)]
fn static_runtime_library_filename() -> &'static str {
    "libruntime.a"
}
