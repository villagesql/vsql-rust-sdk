//! `cargo vsql inspect`: load a `.veb` the way the server does and report what it registers.
//!
//! Nothing in the archive records the protocol, functions, types, or capabilities
//! statically: the extension builds its `vef_registration_t` at run time inside
//! `vef_register`. So the tool extracts `lib/<name>.so`, loads it, calls `vef_register`
//! with a `vef_register_arg_t` that looks like a server built from the headers vendored
//! in `villagesql-sys`, copies the returned registration into plain Rust values, and calls
//! `vef_unregister`.
//!
//! It never calls `on_init`, never populates a capability, and never invokes a VDF, but
//! it does run the extension's own registration code inside this process.

use std::ffi::{c_char, CStr, OsStr};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use anyhow::{bail, Context, Result};
use villagesql_sys as sys;

/// The protocol these bindings were generated from: `VEF_PROTOCOL_CURRENT` in
/// `include/villagesql/abi/types.h`. bindgen drops that `#define` because it expands to
/// an enumerator, so it is restated here; a test checks it against the header.
const CURRENT_PROTOCOL: sys::vef_protocol_t = sys::vef_protocol_t_VEF_PROTOCOL_6;

/// What the tool tells the extension about the server it stands in for. The VEF version is
/// the server release the vendored headers came from; the `MySQL` version is the server line
/// that release is built on.
const MYSQL_VERSION: sys::vef_version_t = sys::vef_version_t {
    major: 8,
    minor: 4,
    patch: 0,
    extra: std::ptr::null(),
};

type RegisterFn =
    unsafe extern "C" fn(*mut sys::vef_register_arg_t) -> *mut sys::vef_registration_t;
type UnregisterFn =
    unsafe extern "C" fn(*mut sys::vef_unregister_arg_t, *mut sys::vef_registration_t);

/// Arguments to `cargo vsql inspect`.
#[derive(clap::Args)]
pub struct Opts {
    /// `.veb` archives to inspect, or directories to scan for them
    #[arg(required = true, value_name = "PATH")]
    pub paths: Vec<PathBuf>,

    /// VEF protocol to offer the extension, as the server would. Defaults to the newest
    /// protocol this tool's headers define.
    #[arg(long, value_name = "N")]
    pub protocol: Option<u32>,

    /// Print one line per archive instead of the full report
    #[arg(long)]
    pub brief: bool,
}

// ── Archive ───────────────────────────────────────────────────────────────────

/// The two members of a `.veb` the server reads.
#[derive(Debug)]
struct Veb {
    manifest: serde_json::Value,
    /// Archive path of the shared library, e.g. `lib/vsql_rot13.so`.
    lib_member: String,
    lib: Vec<u8>,
}

fn read_veb<R: Read>(reader: R) -> Result<Veb> {
    let mut archive = tar::Archive::new(reader);
    let mut manifest: Option<serde_json::Value> = None;
    let mut lib: Option<(String, Vec<u8>)> = None;

    for entry in archive.entries().context("reading tar entries")? {
        let mut entry = entry.context("reading tar entry")?;
        // Normalise `./manifest.json` to `manifest.json`.
        let member: PathBuf = entry
            .path()
            .context("reading tar entry path")?
            .components()
            .filter(|c| !matches!(c, Component::CurDir))
            .collect();

        if member == Path::new("manifest.json") {
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes)?;
            manifest = Some(serde_json::from_slice(&bytes).context("parsing manifest.json")?);
        } else if member.parent() == Some(Path::new("lib"))
            && member.extension() == Some(OsStr::new("so"))
        {
            if let Some((first, _)) = &lib {
                bail!(
                    "more than one shared library: {first} and {}",
                    member.display()
                );
            }
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes)?;
            lib = Some((member.display().to_string(), bytes));
        }
    }

    let Some(manifest) = manifest else {
        bail!("no manifest.json in archive");
    };
    let Some((lib_member, lib)) = lib else {
        bail!("no lib/<name>.so in archive");
    };
    Ok(Veb {
        manifest,
        lib_member,
        lib,
    })
}

static TEMP_COUNTER: AtomicU32 = AtomicU32::new(0);

/// A directory removed on drop.
struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Write the archive's shared library to a fresh temporary directory so it can be loaded.
fn write_temp_lib(veb: &Veb) -> Result<(TempDir, PathBuf)> {
    let dir = std::env::temp_dir().join(format!(
        "cargo-vsql-inspect-{}-{}",
        std::process::id(),
        TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    // Refuse a directory that already exists, so nobody can plant one at the guessable
    // name, and keep the extracted library private to this user.
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(&dir)
        .with_context(|| format!("creating {}", dir.display()))?;
    let temp = TempDir(dir);

    let file_name = Path::new(&veb.lib_member)
        .file_name()
        .unwrap_or_else(|| OsStr::new("extension.so"));
    let path = temp.0.join(file_name);
    std::fs::write(&path, &veb.lib).with_context(|| format!("writing {}", path.display()))?;
    Ok((temp, path))
}

// ── Registration ──────────────────────────────────────────────────────────────

/// Everything the tool reads out of a `vef_registration_t`, copied into owned values so
/// the registration can be handed back to `vef_unregister` before anything is printed.
struct Registration {
    protocol: u32,
    error_msg: Option<String>,
    sdk_version: String,
    deprecated_name: Option<String>,
    deprecated_version: Option<String>,
    funcs: Vec<Func>,
    types: Vec<TypeDesc>,
    capabilities: Vec<Capability>,
    on_init: bool,
    on_deinit: bool,
}

struct Func {
    protocol: u32,
    name: String,
    /// `(STRING, INT) -> STRING`, or `(...)` for varargs.
    signature: String,
    attrs: Vec<String>,
}

struct TypeDesc {
    protocol: u32,
    name: String,
    attrs: Vec<String>,
}

struct Capability {
    name: String,
    vtable_hash: Option<String>,
    has_vtable_dest: bool,
    config_hash: Option<String>,
    has_config: bool,
}

/// Copy a NUL-terminated C string, or `None` for a null pointer.
///
/// `p` must be null or point to a NUL-terminated string that outlives the call.
unsafe fn cstr(p: *const c_char) -> Option<String> {
    if p.is_null() {
        None
    } else {
        Some(CStr::from_ptr(p).to_string_lossy().into_owned())
    }
}

/// `major.minor.patch`, with `-extra` appended when the extension set one.
///
/// `v.extra` must be null or point to a NUL-terminated string.
unsafe fn version_string(v: &sys::vef_version_t) -> String {
    let mut out = format!("{}.{}.{}", v.major, v.minor, v.patch);
    if let Some(extra) = cstr(v.extra) {
        if !extra.is_empty() {
            out.push('-');
            out.push_str(&extra);
        }
    }
    out
}

/// `t.custom_type` must be null or point to a NUL-terminated string.
unsafe fn type_name(t: &sys::vef_type_t) -> String {
    match t.id {
        sys::vef_type_id_VEF_TYPE_STRING => "STRING".to_string(),
        sys::vef_type_id_VEF_TYPE_REAL => "REAL".to_string(),
        sys::vef_type_id_VEF_TYPE_INT => "INT".to_string(),
        sys::vef_type_id_VEF_TYPE_BINARY => "BINARY".to_string(),
        sys::vef_type_id_VEF_TYPE_CUSTOM => match cstr(t.custom_type) {
            Some(name) => format!("CUSTOM({name})"),
            None => "CUSTOM(?)".to_string(),
        },
        other => format!("TYPE#{other}"),
    }
}

/// `sig` must be null or point to a valid signature whose `params` array holds
/// `param_count` entries (unless it is the varargs sentinel).
unsafe fn signature_string(sig: *const sys::vef_signature_t) -> String {
    let Some(sig) = sig.as_ref() else {
        return "<no signature>".to_string();
    };
    let params = if sig.param_count == sys::VEF_PARAM_VARARGS {
        "...".to_string()
    } else if sig.params.is_null() {
        if sig.param_count == 0 {
            String::new()
        } else {
            "?".to_string()
        }
    } else {
        std::slice::from_raw_parts(sig.params, sig.param_count as usize)
            .iter()
            .map(|p| type_name(p))
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!("({params}) -> {}", type_name(&sig.return_type))
}

/// Push `key=value` when `value` is a non-null C string.
unsafe fn push_name(attrs: &mut Vec<String>, key: &str, value: *const c_char) {
    if let Some(value) = cstr(value) {
        attrs.push(format!("{key}={value}"));
    }
}

/// Read one function descriptor. Fields added by a later protocol exist only in the
/// layout of the protocol the descriptor declares, so they are read only when it is new
/// enough; reading them from an older descriptor would run off its allocation.
unsafe fn read_func(func: &sys::vef_func_desc_t) -> Func {
    let mut attrs = Vec::new();
    if func.vdf.is_none() {
        attrs.push("vdf=NULL".to_string());
    }
    if func.prerun.is_some() {
        attrs.push("prerun".to_string());
    }
    if func.postrun.is_some() {
        attrs.push("postrun".to_string());
    }
    if func.buffer_size != 0 {
        attrs.push(format!("buffer_size={}", func.buffer_size));
    }
    if func.protocol >= sys::vef_protocol_t_VEF_PROTOCOL_3 {
        if func.deterministic {
            attrs.push("deterministic".to_string());
        }
        match (func.clear.is_some(), func.accumulate.is_some()) {
            (true, true) => attrs.push("aggregate".to_string()),
            (true, false) => attrs.push("clear-without-accumulate".to_string()),
            (false, true) => attrs.push("accumulate-without-clear".to_string()),
            (false, false) => {}
        }
    }
    if func.protocol >= sys::vef_protocol_t_VEF_PROTOCOL_5 {
        if func.max_result_length != 0 {
            attrs.push(format!("max_result_length={}", func.max_result_length));
        }
        if func.bind_and_check_types.is_some() {
            attrs.push("bind_and_check_types".to_string());
        }
    }
    Func {
        protocol: func.protocol,
        name: cstr(func.name).unwrap_or_else(|| "<unnamed>".to_string()),
        signature: signature_string(func.signature.cast_const()),
        attrs,
    }
}

/// Read one type descriptor, gating fields by its declared protocol as [`read_func`] does.
unsafe fn read_type(ty: &sys::vef_type_desc_t) -> TypeDesc {
    let mut attrs = vec![
        format!("persisted_length={}", ty.persisted_length),
        format!("max_decode_buffer_length={}", ty.max_decode_buffer_length),
    ];
    let native: Vec<&str> = [
        ("encode", ty.encode_func.is_some()),
        ("decode", ty.decode_func.is_some()),
        ("compare", ty.compare_func.is_some()),
        ("hash", ty.hash_func.is_some()),
    ]
    .into_iter()
    .filter(|(_, set)| *set)
    .map(|(name, _)| name)
    .collect();
    if !native.is_empty() {
        attrs.push(format!("native_funcs={}", native.join(",")));
    }
    if ty.protocol >= sys::vef_protocol_t_VEF_PROTOCOL_3 {
        push_name(&mut attrs, "encode_vdf", ty.encode_vdf_name);
        push_name(&mut attrs, "decode_vdf", ty.decode_vdf_name);
        push_name(&mut attrs, "compare_vdf", ty.compare_vdf_name);
        push_name(&mut attrs, "hash_vdf", ty.hash_vdf_name);
        push_name(&mut attrs, "int_to_params_vdf", ty.int_to_params_vdf_name);
        push_name(&mut attrs, "resolve_params_vdf", ty.resolve_params_vdf_name);
        push_name(
            &mut attrs,
            "intrinsic_default_vdf",
            ty.intrinsic_default_vdf_name,
        );
        if let Some(default) = cstr(ty.intrinsic_default_str) {
            attrs.push(format!("intrinsic_default_str={default:?}"));
        }
        if ty.max_persisted_length != 0 {
            attrs.push(format!("max_persisted_length={}", ty.max_persisted_length));
        }
    }
    if ty.protocol >= sys::vef_protocol_t_VEF_PROTOCOL_5 {
        if ty.variable_length {
            attrs.push("variable_length".to_string());
        }
        push_name(&mut attrs, "from_binary_vdf", ty.from_binary_vdf_name);
        push_name(&mut attrs, "to_binary_vdf", ty.to_binary_vdf_name);
    }
    TypeDesc {
        protocol: ty.protocol,
        name: cstr(ty.name).unwrap_or_else(|| "<unnamed>".to_string()),
        attrs,
    }
}

/// `cap`'s string fields must be null or point to NUL-terminated strings.
unsafe fn read_capability(cap: &sys::vef_required_capability_t) -> Capability {
    Capability {
        name: cstr(cap.name).unwrap_or_else(|| "<unnamed>".to_string()),
        vtable_hash: cstr(cap.vtable_hash),
        has_vtable_dest: !cap.vtable_dest.is_null(),
        config_hash: cstr(cap.capability_config_hash),
        has_config: !cap.capability_config.is_null(),
    }
}

/// View an array of `count` pointers to descriptors as references, treating a null array
/// as empty and skipping null entries.
///
/// `array` must be null or point to `count` readable pointers, each null or valid.
unsafe fn descriptors<'a, T>(array: *const *mut T, count: u32) -> Vec<&'a T> {
    if array.is_null() || count == 0 {
        return Vec::new();
    }
    std::slice::from_raw_parts(array, count as usize)
        .iter()
        .filter_map(|&p| p.as_ref())
        .collect()
}

/// Copy a registration out. Fields past the protocol-1 layout are read only when the
/// registration's protocol says they exist; fields newer than this tool's headers are
/// never touched, and every struct version is a prefix of the next, so a newer
/// registration is still read correctly as far as it goes.
unsafe fn read_registration(reg: &sys::vef_registration_t) -> Registration {
    let funcs = descriptors(reg.funcs.cast_const(), reg.func_count)
        .into_iter()
        .map(|f| read_func(f))
        .collect();
    let types = descriptors(reg.types.cast_const(), reg.type_count)
        .into_iter()
        .map(|t| read_type(t))
        .collect();

    let capabilities = if reg.protocol >= sys::vef_protocol_t_VEF_PROTOCOL_3
        && !reg.required_capabilities.is_null()
    {
        std::slice::from_raw_parts(
            reg.required_capabilities,
            reg.required_capability_count as usize,
        )
        .iter()
        .map(|c| read_capability(c))
        .collect()
    } else {
        Vec::new()
    };

    let (on_init, on_deinit) = if reg.protocol >= sys::vef_protocol_t_VEF_PROTOCOL_5 {
        (reg.on_init.is_some(), reg.on_deinit.is_some())
    } else {
        (false, false)
    };

    Registration {
        protocol: reg.protocol,
        error_msg: cstr(reg.error_msg),
        sdk_version: version_string(&reg.sdk_version),
        deprecated_name: cstr(reg.deprecated_extension_name),
        deprecated_version: cstr(reg.deprecated_extension_version),
        funcs,
        types,
        capabilities,
        on_init,
        on_deinit,
    }
}

/// Load the library with its undefined symbols resolved up front, so a library that
/// needs something this process does not provide fails here with a clear message
/// rather than inside `vef_register`.
unsafe fn open_library(path: &Path) -> Result<libloading::Library> {
    #[cfg(unix)]
    {
        use libloading::os::unix::{Library, RTLD_LOCAL, RTLD_NOW};
        let lib = Library::open(Some(path), RTLD_NOW | RTLD_LOCAL)
            .with_context(|| format!("loading {}", path.display()))?;
        Ok(lib.into())
    }
    #[cfg(not(unix))]
    {
        libloading::Library::new(path).with_context(|| format!("loading {}", path.display()))
    }
}

/// Load the library, run its `vef_register` as a server offering `offered`, copy the
/// result out, and hand it back through `vef_unregister`.
fn register_through(lib_path: &Path, offered: u32) -> Result<Registration> {
    // SAFETY: loading the library runs its initialisers and `vef_register` runs the
    // extension's registration code; that is the purpose of the tool. The registration
    // it returns is only read, then given back to `vef_unregister` before the library
    // is unloaded.
    unsafe {
        let lib = open_library(lib_path)?;
        let register: libloading::Symbol<RegisterFn> = lib
            .get(sys::VEF_REGISTER_FUNC_NAME)
            .context("the library does not export vef_register")?;
        let unregister: libloading::Symbol<UnregisterFn> =
            lib.get(sys::VEF_UNREGISTER_FUNC_NAME)
                .context("the library does not export vef_unregister")?;

        let mut register_arg = sys::vef_register_arg_t {
            protocol: offered,
            mysql_version: MYSQL_VERSION,
            vef_version: sys::SDK_VERSION,
        };
        let registration = register(&raw mut register_arg);
        if registration.is_null() {
            bail!("vef_register returned NULL");
        }
        let copy = read_registration(&*registration);

        let mut unregister_arg = sys::vef_unregister_arg_t {
            protocol: offered.min(copy.protocol),
        };
        unregister(&raw mut unregister_arg, registration);
        Ok(copy)
    }
}

// ── Reporting ─────────────────────────────────────────────────────────────────

/// One inspected archive.
struct Report {
    file: PathBuf,
    manifest: serde_json::Value,
    lib_member: String,
    lib_size: usize,
    registration: Registration,
}

fn inspect_one(file: &Path, offered: u32) -> Result<Report> {
    let reader =
        std::fs::File::open(file).with_context(|| format!("opening {}", file.display()))?;
    let veb = read_veb(reader)?;
    let (temp, lib_path) = write_temp_lib(&veb)?;
    let registration = register_through(&lib_path, offered)?;
    drop(temp);
    Ok(Report {
        file: file.to_path_buf(),
        manifest: veb.manifest,
        lib_member: veb.lib_member,
        lib_size: veb.lib.len(),
        registration,
    })
}

/// What the server does with a registration at this protocol, per the comments on
/// `vef_protocol_t` in `types.h`.
fn protocol_status(protocol: u32) -> &'static str {
    match protocol {
        0 => "not used; the server rejects it",
        1 => "stable since server 0.0.1; deprecated, still accepted",
        2 | 4 => "unstable development version; the server rejects it",
        3 => "stable since server 0.0.4; deprecated, still accepted",
        5 => "stable since server 0.0.8",
        6 => "under development, not stable",
        _ => "newer than this tool's headers",
    }
}

/// Manifest keys in display order: the documented ones first, then anything else.
fn manifest_entries(manifest: &serde_json::Value) -> Vec<(String, String)> {
    const FIRST: [&str; 5] = ["name", "version", "description", "author", "license"];
    let Some(map) = manifest.as_object() else {
        return vec![("manifest".to_string(), manifest.to_string())];
    };
    let mut out = Vec::new();
    for key in FIRST {
        if let Some(value) = map.get(key) {
            out.push((key.to_string(), json_scalar(value)));
        }
    }
    for (key, value) in map {
        if !FIRST.contains(&key.as_str()) {
            out.push((key.clone(), json_scalar(value)));
        }
    }
    out
}

fn json_scalar(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn manifest_str(manifest: &serde_json::Value, key: &str) -> String {
    manifest
        .get(key)
        .and_then(serde_json::Value::as_str)
        .unwrap_or("?")
        .to_string()
}

/// `[a, b, c]`, with the descriptor's own protocol first when it differs from the
/// registration's; empty when there is nothing to say.
fn attrs_string(attrs: &[String], protocol: u32, registration_protocol: u32) -> String {
    let mut all = Vec::with_capacity(attrs.len() + 1);
    if protocol != registration_protocol {
        all.push(format!("protocol={protocol}"));
    }
    all.extend(attrs.iter().cloned());
    if all.is_empty() {
        String::new()
    } else {
        format!("  [{}]", all.join(", "))
    }
}

fn render_full(report: &Report) -> String {
    let reg = &report.registration;
    let mut lines = vec![report.file.display().to_string(), "  manifest".to_string()];
    for (key, value) in manifest_entries(&report.manifest) {
        lines.push(format!("    {key}: {value}"));
    }
    lines.push(format!(
        "  library: {} ({} bytes)",
        report.lib_member, report.lib_size
    ));

    lines.push("  registration".to_string());
    lines.push(format!(
        "    protocol: {} ({})",
        reg.protocol,
        protocol_status(reg.protocol)
    ));
    lines.push(format!("    sdk_version: {}", reg.sdk_version));
    if let Some(msg) = &reg.error_msg {
        lines.push(format!("    error_msg: {msg}"));
    }
    if let Some(name) = &reg.deprecated_name {
        lines.push(format!("    deprecated_extension_name: {name}"));
    }
    if let Some(version) = &reg.deprecated_version {
        lines.push(format!("    deprecated_extension_version: {version}"));
    }
    let hooks = match (reg.on_init, reg.on_deinit) {
        (true, true) => "on_init, on_deinit",
        (true, false) => "on_init only (the server rejects this)",
        (false, true) => "on_deinit only (the server rejects this)",
        (false, false) => "none",
    };
    lines.push(format!("    load hooks: {hooks}"));

    lines.push(format!("    functions: {}", reg.funcs.len()));
    for func in &reg.funcs {
        lines.push(format!(
            "      {}{}{}",
            func.name,
            func.signature,
            attrs_string(&func.attrs, func.protocol, reg.protocol)
        ));
    }

    lines.push(format!("    types: {}", reg.types.len()));
    for ty in &reg.types {
        lines.push(format!(
            "      {}{}",
            ty.name,
            attrs_string(&ty.attrs, ty.protocol, reg.protocol)
        ));
    }

    let preview = if reg.capabilities.is_empty() {
        ""
    } else {
        " (preview: the server loads this only with vsql_allow_preview_extensions=ON)"
    };
    lines.push(format!(
        "    required capabilities: {}{preview}",
        reg.capabilities.len()
    ));
    for cap in &reg.capabilities {
        let mut line = format!(
            "      {}  vtable_hash={}  config_hash={}  config={}",
            cap.name,
            cap.vtable_hash.as_deref().unwrap_or("NULL"),
            cap.config_hash.as_deref().unwrap_or("NULL"),
            if cap.has_config { "set" } else { "NULL" }
        );
        if !cap.has_vtable_dest {
            line.push_str("  vtable_dest=NULL");
        }
        lines.push(line);
    }
    lines.join("\n")
}

fn brief_header() -> String {
    format!(
        "{:<28} {:<22} {:<8} {:>5} {:>5} {:>5}  CAPABILITIES",
        "FILE", "NAME", "VERSION", "PROTO", "FUNCS", "TYPES"
    )
}

fn render_brief(report: &Report) -> String {
    let reg = &report.registration;
    let file = report.file.file_name().map_or_else(
        || report.file.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    );
    let name = manifest_str(&report.manifest, "name");
    let version = manifest_str(&report.manifest, "version");
    let caps = if reg.capabilities.is_empty() {
        "-".to_string()
    } else {
        reg.capabilities
            .iter()
            .map(|cap| cap.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut line = format!(
        "{file:<28} {name:<22} {version:<8} {:>5} {:>5} {:>5}  {caps}",
        reg.protocol,
        reg.funcs.len(),
        reg.types.len()
    );
    if let Some(msg) = &reg.error_msg {
        line.push_str("  ERROR: ");
        line.push_str(msg);
    }
    line
}

// ── Entry point ───────────────────────────────────────────────────────────────

fn is_veb(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("veb"))
}

/// Expand directories to the `.veb` files inside them, in name order.
fn collect_archives(paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for path in paths {
        if path.is_dir() {
            let mut found: Vec<PathBuf> = std::fs::read_dir(path)
                .with_context(|| format!("reading {}", path.display()))?
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|p| is_veb(p))
                .collect();
            if found.is_empty() {
                bail!("no .veb archives in {}", path.display());
            }
            found.sort();
            out.extend(found);
        } else {
            out.push(path.clone());
        }
    }
    Ok(out)
}

/// Run `cargo vsql inspect`. Every archive is attempted; the error names how many failed.
pub fn run(opts: &Opts) -> Result<()> {
    let offered = opts.protocol.unwrap_or(CURRENT_PROTOCOL);
    let archives = collect_archives(&opts.paths)?;

    // SAFETY: SDK_VERSION.extra is null.
    let sdk = unsafe { version_string(&sys::SDK_VERSION) };
    println!(
        "cargo vsql inspect: offering VEF protocol {offered}, as a server with headers from release {sdk}"
    );
    if opts.brief {
        println!("{}", brief_header());
    }

    let mut failures = 0usize;
    for file in &archives {
        match inspect_one(file, offered) {
            Ok(report) if opts.brief => println!("{}", render_brief(&report)),
            Ok(report) => {
                println!();
                println!("{}", render_full(&report));
            }
            Err(err) => {
                failures += 1;
                eprintln!("{}: {err:#}", file.display());
            }
        }
    }

    if failures > 0 {
        bail!(
            "{failures} of {} archives could not be inspected",
            archives.len()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `CURRENT_PROTOCOL` restates `VEF_PROTOCOL_CURRENT`, which bindgen drops; keep them
    /// equal. The header is only present in a source checkout, not in a published crate.
    #[test]
    fn current_protocol_matches_header() {
        let header =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../include/villagesql/abi/types.h");
        let Ok(text) = std::fs::read_to_string(&header) else {
            return;
        };
        let define = text
            .lines()
            .find(|line| line.starts_with("#define VEF_PROTOCOL_CURRENT"))
            .expect("types.h defines VEF_PROTOCOL_CURRENT");
        let value: u32 = define
            .rsplit('_')
            .next()
            .expect("enumerator name ends in a number")
            .trim()
            .parse()
            .expect("enumerator suffix is a number");
        assert_eq!(value, CURRENT_PROTOCOL);
    }

    fn archive(members: &[(&str, &[u8])]) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        for (path, data) in members {
            let mut header = tar::Header::new_gnu();
            header.set_size(u64::try_from(data.len()).unwrap());
            header.set_mode(0o644);
            header.set_cksum();
            builder.append_data(&mut header, *path, *data).unwrap();
        }
        builder.into_inner().unwrap()
    }

    #[test]
    fn reads_manifest_and_library() {
        let bytes = archive(&[
            ("./manifest.json", br#"{"name":"x","version":"1.2.3"}"#),
            ("lib/x.so", b"\x7fELF"),
        ]);
        let veb = read_veb(bytes.as_slice()).unwrap();
        assert_eq!(veb.manifest["name"], "x");
        assert_eq!(veb.lib_member, "lib/x.so");
        assert_eq!(veb.lib, b"\x7fELF");
    }

    #[test]
    fn rejects_archive_without_library() {
        let bytes = archive(&[("manifest.json", b"{}")]);
        let err = read_veb(bytes.as_slice()).unwrap_err();
        assert!(err.to_string().contains("lib/<name>.so"));
    }

    #[test]
    fn rejects_archive_without_manifest() {
        let bytes = archive(&[("lib/x.so", b"\x7fELF")]);
        let err = read_veb(bytes.as_slice()).unwrap_err();
        assert!(err.to_string().contains("manifest.json"));
    }

    fn sample_report() -> Report {
        Report {
            file: PathBuf::from("vsql_ping.veb"),
            manifest: serde_json::json!({"name": "vsql_ping", "version": "0.1.0"}),
            lib_member: "lib/vsql_ping.so".to_string(),
            lib_size: 42,
            registration: Registration {
                protocol: 3,
                error_msg: None,
                sdk_version: "0.0.8".to_string(),
                deprecated_name: None,
                deprecated_version: None,
                funcs: vec![Func {
                    protocol: 3,
                    name: "ping".to_string(),
                    signature: "(STRING) -> STRING".to_string(),
                    attrs: vec!["deterministic".to_string()],
                }],
                types: Vec::new(),
                capabilities: vec![Capability {
                    name: "vsql::preview::ping".to_string(),
                    vtable_hash: Some("ver-1".to_string()),
                    has_vtable_dest: true,
                    config_hash: None,
                    has_config: false,
                }],
                on_init: false,
                on_deinit: false,
            },
        }
    }

    #[test]
    fn full_report_lists_capabilities() {
        let text = render_full(&sample_report());
        assert!(text.contains("protocol: 3 (stable since server 0.0.4"));
        assert!(text.contains("ping(STRING) -> STRING  [deterministic]"));
        assert!(text.contains("required capabilities: 1 (preview"));
        assert!(
            text.contains("vsql::preview::ping  vtable_hash=ver-1  config_hash=NULL  config=NULL")
        );
    }

    #[test]
    fn brief_report_is_one_line() {
        let text = render_brief(&sample_report());
        assert_eq!(text.lines().count(), 1);
        assert!(text.starts_with("vsql_ping.veb"));
        assert!(text.ends_with("vsql::preview::ping"));
    }

    #[test]
    fn attrs_show_protocol_only_when_it_differs() {
        assert_eq!(attrs_string(&[], 3, 3), "");
        assert_eq!(attrs_string(&[], 3, 5), "  [protocol=3]");
        assert_eq!(attrs_string(&["prerun".to_string()], 5, 5), "  [prerun]");
    }
}
