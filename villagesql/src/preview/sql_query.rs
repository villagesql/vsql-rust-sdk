//! Idiomatic wrapper for the `vsql::preview::sql_query` capability.
//! Raw ABI is generated in villagesql-sys.
//!
//! Based on the server header `villagesql/stable_sdk/v3/include/villagesql/
//! abi/preview/sql_query.h`.
//! This is a preview capability. The ABI is version-gated via the 'version' field
//! and may change in future versions.

use std::ffi::{c_char, c_uint, c_ulong, c_void, CStr};
use std::marker::PhantomData;
use std::sync::atomic::{AtomicPtr, Ordering};

use crate::preview::thread_worker::ThreadHandle;
use crate::preview::{Capability, RequiredCapability};
use crate::sys::{
    vef_preview_sql_query_t, vef_sql_diag_t, vef_sql_result_t, vef_sql_session_t,
    VEF_PREVIEW_SQL_QUERY_ABI_VERSION, VEF_PREVIEW_SQL_QUERY_NAME,
};
use crate::sys::{
    vef_sql_diag_severity_t_VEF_SQL_DIAG_NOTE as VEF_SQL_DIAG_NOTE,
    vef_sql_diag_severity_t_VEF_SQL_DIAG_WARNING as VEF_SQL_DIAG_WARNING,
};

const VTABLE_HASH: &[u8] = b"ver-1\0";

/// The `vsql::preview::sql_query` capability. Declare it as a `static` and list
/// it via `requires: [&SQL_QUERY]`. Open a session from inside a thread-worker
/// callback with [`SqlQueryCapability::open`].
pub struct SqlQueryCapability {
    abi_: AtomicPtr<vef_preview_sql_query_t>,
}

impl SqlQueryCapability {
    /// Create the capability in its unpopulated state. `const`, so it can
    /// initialize the `static` the extension declares. The server fills it in at
    /// registration, provided the extension named it in `requires:`.
    #[must_use]
    #[allow(clippy::new_without_default)]
    pub const fn new() -> Self {
        Self {
            abi_: AtomicPtr::new(std::ptr::null_mut()),
        }
    }

    /// Open a SQL session bound to the worker thread. `None` if the capability
    /// wasn't populated (preview off/not requested), the ABI is too old, or
    /// the server failed to open a session.
    #[must_use]
    pub fn open(&self, handle: &ThreadHandle) -> Option<Session<'_>> {
        let vtable = self.abi_.load(Ordering::Acquire);
        if vtable.is_null() {
            return None;
        }
        // SAFETY: non-null slot written by the server at load time. Points to a
        // 'static vef_preview_sql_query_t the server owns.
        let vtable = unsafe { &*vtable };
        if vtable.version < VEF_PREVIEW_SQL_QUERY_ABI_VERSION {
            return None;
        }
        let open = vtable.open_session?;
        // SAFETY: server-provided fn. `handle` is a live worker-thread handle.
        let raw = unsafe { open(handle.as_raw()) };
        if raw.is_null() {
            return None;
        }
        Some(Session {
            vtable,
            handle: raw,
        })
    }
}

impl Capability for &'static SqlQueryCapability {
    fn request(self) -> RequiredCapability {
        RequiredCapability {
            name: VEF_PREVIEW_SQL_QUERY_NAME.as_ptr().cast::<c_char>(),
            vtable_hash: VTABLE_HASH.as_ptr().cast::<c_char>(),
            vtable_dest: self.abi_.as_ptr().cast::<*mut c_void>(),
            capability_config_hash: std::ptr::null(),
            capability_config: std::ptr::null(),
        }
    }
}

/// An open SQL session, bound to the worker thread. Closes automatically on drop.
pub struct Session<'a> {
    vtable: &'a vef_preview_sql_query_t,
    handle: *mut vef_sql_session_t,
}

impl<'a> Session<'a> {
    /// Execute a SQL statement, buffering the full result set. `None` only on
    /// setup failure (no session / OOM). A returned `QueryResult` may still be a
    /// failed query. Check `has_error()` / `error()`.
    #[must_use]
    pub fn execute(&self, sql: &str) -> Option<QueryResult<'a>> {
        let exec = self.vtable.execute?;
        // SAFETY: server-provided fn. sql/len describe a valid UTF-8 buffer.
        let raw = unsafe { exec(self.handle, sql.as_ptr().cast::<c_char>(), sql.len()) };
        if raw.is_null() {
            return None;
        }
        Some(QueryResult {
            vtable: self.vtable,
            handle: raw,
        })
    }

    /// Execute `sql`, invoking `f` once per row without buffering the full
    /// result set. Returns a `QueryResult` that carries diagnostics only (no
    /// buffered rows). Check `has_error()` / `warnings()` on it afterward.
    /// `None` on setup failure.
    pub fn for_each<F>(&self, sql: &str, mut f: F) -> Option<QueryResult<'a>>
    where
        F: FnMut(Row<'_>),
    {
        // C entry point: recover F from ctx, build a Row, call f. On panic we
        // stop (returning false) rather than let it unwind across the C boundary.
        unsafe extern "C" fn trampoline<F: FnMut(Row<'_>)>(
            row: *mut *const c_char,
            lengths: *const c_ulong,
            num_columns: c_uint,
            ctx: *mut c_void,
        ) -> bool {
            // SAFETY: ctx is the &mut F we pass to for_each_row below.
            let f = unsafe { &mut *(ctx.cast::<F>()) };
            let row = Row {
                row,
                lengths,
                num_columns,
                _marker: PhantomData,
            };
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(row))).is_ok()
        }

        let for_each_fn = self.vtable.for_each_row?;

        let ctx = std::ptr::from_mut(&mut f).cast::<c_void>();
        // SAFETY: server-provided fn. sql/len valid. Trampoline and ctx live for
        // the whole call.
        let raw = unsafe {
            for_each_fn(
                self.handle,
                sql.as_ptr().cast::<c_char>(),
                sql.len(),
                Some(trampoline::<F>),
                ctx,
            )
        };
        if raw.is_null() {
            return None;
        }
        Some(QueryResult {
            vtable: self.vtable,
            handle: raw,
        })
    }
}

impl Drop for Session<'_> {
    fn drop(&mut self) {
        if let Some(close) = self.vtable.close_session {
            // SAFETY: `handle` came from open_session and is closed exactly once.
            unsafe { close(self.handle) }
        }
    }
}

/// The outcome of a query: diagnostics, plus (for `execute`) buffered rows.
/// Closes the underlying handle on drop.
pub struct QueryResult<'a> {
    vtable: &'a vef_preview_sql_query_t,
    handle: *mut vef_sql_result_t,
}

impl QueryResult<'_> {
    /// True if the statement produced an error.
    #[must_use]
    pub fn has_error(&self) -> bool {
        self.vtable
            .has_error
            .is_some_and(|f| unsafe { f(self.handle) })
    }

    /// The statement error, if the query failed.
    #[must_use]
    pub fn error(&self) -> Option<Diag> {
        let get = self.vtable.get_error?;
        let mut raw: vef_sql_diag_t = unsafe { std::mem::zeroed() };
        // SAFETY: server fills *raw and returns true iff there is an error.
        if unsafe { get(self.handle, &raw mut raw) } {
            Some(unsafe { Diag::from_raw(&raw) })
        } else {
            None
        }
    }

    /// All warnings/notes attached to the result.
    #[must_use]
    pub fn warnings(&self) -> Vec<Diag> {
        let (Some(count_fn), Some(get_fn)) = (self.vtable.warning_count, self.vtable.get_warning)
        else {
            return Vec::new();
        };
        let n = unsafe { count_fn(self.handle) };
        let mut out = Vec::with_capacity(n as usize);
        for i in 0..n {
            let mut raw: vef_sql_diag_t = unsafe { std::mem::zeroed() };
            if unsafe { get_fn(self.handle, i, &raw mut raw) } {
                out.push(unsafe { Diag::from_raw(&raw) });
            }
        }
        out
    }

    /// Number of columns in the result set.
    #[must_use]
    pub fn num_columns(&self) -> u32 {
        self.vtable
            .num_columns
            .map_or(0, |f| unsafe { f(self.handle) })
    }

    /// Fetch the next row, or `None` when the result set is exhausted.
    ///
    /// The returned `Row` borrows `self`, so it can't outlive the next
    /// `next_row()` call, which is exactly the server's "valid until next
    /// fetch" rule, enforced for free by the borrow checker.
    pub fn next_row(&mut self) -> Option<Row<'_>> {
        let fetch = self.vtable.fetch_row?;
        let mut row: *mut *const c_char = std::ptr::null_mut();
        let mut lengths: *const c_ulong = std::ptr::null();
        // SAFETY: server fills row/lengths and returns true iff a row was fetched.
        let got = unsafe { fetch(self.handle, &raw mut row, &raw mut lengths) };
        if !got {
            return None;
        }
        Some(Row {
            row,
            lengths,
            num_columns: self.num_columns(),
            _marker: PhantomData,
        })
    }
}

impl Drop for QueryResult<'_> {
    fn drop(&mut self) {
        if let Some(close) = self.vtable.close_result {
            // SAFETY: `handle` came from execute/for_each and is closed once.
            unsafe { close(self.handle) }
        }
    }
}

/// One row of a buffered result set. Borrows the [`QueryResult`], so it can't
/// outlive the next `next_row()` call. Its column values are valid only in that
/// window. A `None` column value is SQL NULL.
#[allow(clippy::struct_field_names)] // mirrors the ABI callback's `row`/`lengths`/`num_columns`
pub struct Row<'r> {
    row: *mut *const c_char,
    lengths: *const c_ulong,
    num_columns: u32,
    _marker: PhantomData<&'r ()>,
}

impl<'r> Row<'r> {
    /// Number of columns in this row.
    #[must_use]
    pub fn num_columns(&self) -> u32 {
        self.num_columns
    }

    /// Column `i` as raw bytes. `None` for SQL NULL or out-of-range `i`.
    #[must_use]
    pub fn get_bytes(&self, i: u32) -> Option<&'r [u8]> {
        if i >= self.num_columns {
            return None;
        }
        let i = i as usize;
        // SAFETY: row/lengths are arrays of `num_columns` entries, valid for 'r
        // (the &mut borrow in the next_row blocks a re-fetch for that whole window).
        let ptr = unsafe { *self.row.add(i) };
        if ptr.is_null() {
            return None; // SQL NULL
        }
        let len = usize::try_from(unsafe { *self.lengths.add(i) }).unwrap_or(0);
        Some(unsafe { std::slice::from_raw_parts(ptr.cast::<u8>(), len) })
    }

    /// Column `i` as UTF-8 text. `None` for SQL NULL, out-of-range, or non-UTF-8.
    #[must_use]
    pub fn get_str(&self, i: u32) -> Option<&'r str> {
        std::str::from_utf8(self.get_bytes(i)?).ok()
    }

    /// Column `i` parsed as an integer. `None` for SQL NULL or unparseable text.
    #[must_use]
    pub fn get_int(&self, i: u32) -> Option<i64> {
        self.get_str(i)?.trim().parse().ok()
    }

    /// Column `i` parsed as a float. `None` for SQL NULL or unparseable text.
    #[must_use]
    pub fn get_real(&self, i: u32) -> Option<f64> {
        self.get_str(i)?.trim().parse().ok()
    }
}

/// Severity of a SQL diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagSeverity {
    /// Informational. The statement succeeded.
    Note,
    /// The statement succeeded, but the server has something to report.
    Warning,
    /// The statement failed.
    Error,
}

/// One diagnostic: the statement error, or a warning/note. Strings are copied
/// out of server storage (owned), so a `Diag` outlives the `QueryResult`.
#[derive(Debug, Clone)]
pub struct Diag {
    /// The MySQL error number, such as `1146` for an unknown table.
    pub errno: u32,
    /// Whether this is the statement's error, or a warning or note beside it.
    pub severity: DiagSeverity,
    /// The five-character SQLSTATE, such as `42S02`.
    pub sqlstate: String,
    /// The human-readable message text.
    pub message: String,
}

impl Diag {
    /// # Safety
    /// `raw` must be a diagnostic the server just filled. Its pointers are valid
    /// for the duration of this call.
    unsafe fn from_raw(raw: &vef_sql_diag_t) -> Self {
        let severity = match raw.severity {
            VEF_SQL_DIAG_NOTE => DiagSeverity::Note,
            VEF_SQL_DIAG_WARNING => DiagSeverity::Warning,
            _ => DiagSeverity::Error, // VEF_SQL_DIAG_ERROR and any unknown val
        };
        // sqlstate: NUL-terminated, never null
        let sqlstate = if raw.sqlstate.is_null() {
            String::new()
        } else {
            CStr::from_ptr(raw.sqlstate).to_string_lossy().into_owned()
        };
        // message: `message_len` bytes, never null but may be empty.
        let message = if raw.message.is_null() {
            String::new()
        } else {
            let bytes = std::slice::from_raw_parts(raw.message.cast::<u8>(), raw.message_len);
            String::from_utf8_lossy(bytes).into_owned()
        };
        Self {
            errno: raw.errno_,
            severity,
            sqlstate,
            message,
        }
    }
}
