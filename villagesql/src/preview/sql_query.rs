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

/// Why a `sql_query` call couldn't produce a session or a result.
///
/// A statement that ran but failed (bad SQL, missing table) is not an error
/// here. It comes back as `Ok(QueryResult)` with [`QueryResult::has_error`]
/// set, so its warnings can still be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SqlQueryError {
    /// The capability was not wired up. The server never populated the
    /// vtable, a needed function pointer is null, or its ABI version is
    /// older than this SDK requires.
    CapabilityUnavailable,
    /// The server could not open the session or start the statement, for
    /// example because it ran out of memory.
    Failed,
}

impl std::fmt::Display for SqlQueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            SqlQueryError::CapabilityUnavailable => "sql_query capability is unavailable",
            SqlQueryError::Failed => "sql_query operation failed",
        })
    }
}

impl std::error::Error for SqlQueryError {}

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

    /// Open a SQL session bound to the worker thread.
    ///
    /// # Errors
    /// - [`SqlQueryError::CapabilityUnavailable`]: preview disabled or not
    ///   requested, the server's ABI is too old, or `open_session` is unset.
    /// - [`SqlQueryError::Failed`]: the server could not open a session.
    pub fn open(&self, handle: &ThreadHandle) -> Result<Session<'_>, SqlQueryError> {
        let vtable = self.abi_.load(Ordering::Acquire);
        if vtable.is_null() {
            return Err(SqlQueryError::CapabilityUnavailable);
        }
        // SAFETY: non-null slot written by the server at load time. Points to a
        // 'static vef_preview_sql_query_t the server owns.
        let vtable = unsafe { &*vtable };
        if vtable.version < VEF_PREVIEW_SQL_QUERY_ABI_VERSION {
            return Err(SqlQueryError::CapabilityUnavailable);
        }
        let open = vtable
            .open_session
            .ok_or(SqlQueryError::CapabilityUnavailable)?;
        // SAFETY: server-provided fn. `handle` is a live worker-thread handle.
        let raw = unsafe { open(handle.as_raw()) };
        if raw.is_null() {
            return Err(SqlQueryError::Failed);
        }
        Ok(Session {
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

/// What `for_each` passes to the C row callback through `ctx`: the author's
/// closure, plus a slot for a panic caught mid-iteration. This allows `for_each`
/// to re-raise the panic once the server call returns.
struct ForEachCtx<F> {
    f: F,
    panic: Option<Box<dyn std::any::Any + Send>>,
}

/// An open SQL session, bound to the worker thread. Closes automatically on drop.
pub struct Session<'a> {
    vtable: &'a vef_preview_sql_query_t,
    handle: *mut vef_sql_session_t,
}

impl<'a> Session<'a> {
    /// Execute a SQL statement, buffering the full result set.
    ///
    /// An `Ok` result may still hold a failed statement: check
    /// [`QueryResult::has_error`] / [`QueryResult::error`].
    ///
    /// # Errors
    /// - [`SqlQueryError::CapabilityUnavailable`]: the server doesn't provide
    ///   `execute`.
    /// - [`SqlQueryError::Failed`]: the server couldn't start the statement,
    ///   for example because it ran out of memory.
    pub fn execute(&self, sql: &str) -> Result<QueryResult<'a>, SqlQueryError> {
        let exec = self
            .vtable
            .execute
            .ok_or(SqlQueryError::CapabilityUnavailable)?;
        // SAFETY: server-provided fn. sql/len describe a valid UTF-8 buffer.
        let raw = unsafe { exec(self.handle, sql.as_ptr().cast::<c_char>(), sql.len()) };
        if raw.is_null() {
            return Err(SqlQueryError::Failed);
        }
        Ok(QueryResult {
            vtable: self.vtable,
            handle: raw,
        })
    }

    /// Execute `sql`, invoking `f` once per row without buffering the full
    /// result set. Returns a `QueryResult` that carries diagnostics only (no
    /// buffered rows). Check `has_error()` / `warnings()` on it afterward.
    ///
    /// # Errors
    /// - [`SqlQueryError::CapabilityUnavailable`]: the server doesn't provide
    ///   `for_each_row`.
    /// - [`SqlQueryError::Failed`]: the server couldn't start the statement.
    ///
    /// # Panics
    /// If `f` panics, iteration stops and the panic propagates out of
    /// `for_each` once the server call returns. In a thread worker it then ends
    /// the work function like any other panic; see
    /// [`WakeupReason`](super::thread_worker::WakeupReason) for how the worker
    /// handles that.
    pub fn for_each<F>(&self, sql: &str, f: F) -> Result<QueryResult<'a>, SqlQueryError>
    where
        F: FnMut(Row<'_>),
    {
        // C entry point: recover the ForEachCtx from ctx, build a Row, call f.
        // On panic we stash the payload and stop (returning false) rather than
        // let it unwind across the C boundary. for_each re-raises it afterward.
        unsafe extern "C" fn trampoline<F: FnMut(Row<'_>)>(
            row: *mut *const c_char,
            lengths: *const c_ulong,
            num_columns: c_uint,
            ctx: *mut c_void,
        ) -> bool {
            // SAFETY: ctx is the &mut ForEachCtx<F> we pass to for_each_row below.
            let state = unsafe { &mut *(ctx.cast::<ForEachCtx<F>>()) };
            let row = Row {
                row,
                lengths,
                num_columns,
                _marker: PhantomData,
            };
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (state.f)(row))) {
                Ok(()) => true,
                Err(payload) => {
                    state.panic = Some(payload);
                    false
                }
            }
        }

        let for_each_fn = self
            .vtable
            .for_each_row
            .ok_or(SqlQueryError::CapabilityUnavailable)?;

        let mut state = ForEachCtx { f, panic: None };
        let ctx = std::ptr::from_mut(&mut state).cast::<c_void>();
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

        // Wrap the handle first: if we re-raise below, QueryResult's Drop still
        // closes it during the unwind.
        let result = if raw.is_null() {
            Err(SqlQueryError::Failed)
        } else {
            Ok(QueryResult {
                vtable: self.vtable,
                handle: raw,
            })
        };
        if let Some(payload) = state.panic {
            // Unwinding is safe here. The thread worker's catch_unwind catches
            // it.
            std::panic::resume_unwind(payload);
        }
        result
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
    /// The `MySQL` error number, such as `1146` for an unknown table.
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::sync::atomic::AtomicUsize;

    // How many times fake_close_result has run.
    static CLOSED: AtomicUsize = AtomicUsize::new(0);

    // Stand-in for the server's close_result: just counts calls.
    unsafe extern "C" fn fake_close_result(_result: *mut vef_sql_result_t) {
        CLOSED.fetch_add(1, Ordering::SeqCst);
    }

    // Stand-in for the server's for_each_row: feeds three rows to `cb`,
    // stopping early if `cb` returns false, like the real one does.
    unsafe extern "C" fn fake_for_each_row(
        _session: *mut vef_sql_session_t,
        _sql: *const c_char,
        _sql_len: usize,
        cb: crate::sys::vef_sql_row_cb,
        ctx: *mut c_void,
    ) -> *mut vef_sql_result_t {
        let cb = cb.expect("for_each passes a callback");
        for _ in 0..3 {
            if !unsafe { cb(std::ptr::null_mut(), std::ptr::null(), 0, ctx) } {
                break;
            }
        }
        std::ptr::NonNull::dangling().as_ptr()
    }

    #[test]
    fn for_each_propagates_closure_panic() {
        // SAFETY: an all-zero vtable is valid. Every fn pointer is None.
        let mut vtable: vef_preview_sql_query_t = unsafe { std::mem::zeroed() };
        vtable.for_each_row = Some(fake_for_each_row);
        vtable.close_result = Some(fake_close_result);
        let session = Session {
            vtable: &vtable,
            handle: std::ptr::null_mut(),
        };

        let mut calls = 0;
        let caught = catch_unwind(AssertUnwindSafe(|| {
            let _ = session.for_each("SELECT 1", |_row| {
                calls += 1;
                panic!("boom");
            });
        }));

        let payload = caught.expect_err("the closure's panic propagates out of for_each");
        assert_eq!(payload.downcast_ref::<&str>(), Some(&"boom"));
        assert_eq!(calls, 1, "iteration stops at the first panic");
        assert_eq!(
            CLOSED.load(Ordering::SeqCst),
            1,
            "the result handle is closed during the unwind"
        );
    }
}
