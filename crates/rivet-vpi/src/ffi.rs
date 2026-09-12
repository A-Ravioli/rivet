//! Minimal hand-written bindings to IEEE 1800 VPI (`vpi_user.h`).
//!
//! Only the subset Rivet uses. Symbols resolve at load time against the
//! simulator (Icarus `vvp`, Verilator's `verilated_vpi`, and so on).

#![allow(non_camel_case_types, non_upper_case_globals, dead_code)]

use std::os::raw::{c_char, c_int, c_uint, c_void};

pub type vpiHandle = *mut c_void;
pub type PLI_INT32 = c_int;
pub type PLI_UINT32 = c_uint;
pub type PLI_BYTE8 = c_char;

// Object types
pub const vpiBegin: i32 = 4;
pub const vpiNamedBegin: i32 = 33;
pub const vpiConstant: i32 = 7;
pub const vpiIntegerVar: i32 = 25;
pub const vpiMemory: i32 = 29;
pub const vpiMemoryWord: i32 = 30;
pub const vpiModule: i32 = 32;
pub const vpiNamedEvent: i32 = 34;
pub const vpiNet: i32 = 36;
pub const vpiParameter: i32 = 41;
pub const vpiPartSelect: i32 = 42;
pub const vpiPort: i32 = 44;
pub const vpiRealVar: i32 = 47;
pub const vpiReg: i32 = 48;
pub const vpiRegBit: i32 = 49;
pub const vpiTimeVar: i32 = 63;
pub const vpiVarSelect: i32 = 68;
pub const vpiBitSelect: i32 = 106;
pub const vpiModuleArray: i32 = 112;
pub const vpiNetArray: i32 = 114;
pub const vpiRange: i32 = 115;
pub const vpiRegArray: i32 = 116;
pub const vpiGenScopeArray: i32 = 133;
pub const vpiGenScope: i32 = 134;
pub const vpiInterface: i32 = 601;
pub const vpiProgram: i32 = 602;
pub const vpiInterfaceArray: i32 = 603;
pub const vpiArrayMember: i32 = 607;
pub const vpiLongIntVar: i32 = 610;
pub const vpiShortIntVar: i32 = 611;
pub const vpiIntVar: i32 = 612;
pub const vpiByteVar: i32 = 614;
pub const vpiStringVar: i32 = 616;
pub const vpiEnumVar: i32 = 617;
pub const vpiStructVar: i32 = 618;
pub const vpiStructNet: i32 = 619;
pub const vpiBitVar: i32 = 620;
pub const vpiPackedArrayVar: i32 = 623;
pub const vpiPackage: i32 = 600;

// One-to-many relationships / iteration
pub const vpiInternalScope: i32 = 92;
pub const vpiScope: i32 = 84;
pub const vpiVariables: i32 = 100;
pub const vpiInstance: i32 = 745;

// Properties
pub const vpiUndefined: i32 = -1;
pub const vpiType: i32 = 1;
pub const vpiName: i32 = 2;
pub const vpiFullName: i32 = 3;
pub const vpiSize: i32 = 4;
pub const vpiDefName: i32 = 9;
pub const vpiTimeUnit: i32 = 11;
pub const vpiTimePrecision: i32 = 12;
pub const vpiConstType: i32 = 40;
pub const vpiSigned: i32 = 65;
pub const vpiLeftRange: i32 = 79;
pub const vpiRightRange: i32 = 83;
pub const vpiPacked: i32 = 630;

// Constant types
pub const vpiDecConst: i32 = 1;
pub const vpiRealConst: i32 = 2;
pub const vpiBinaryConst: i32 = 3;
pub const vpiOctConst: i32 = 4;
pub const vpiHexConst: i32 = 5;
pub const vpiStringConst: i32 = 6;
pub const vpiIntConst: i32 = 7;

// Value formats
pub const vpiBinStrVal: i32 = 1;
pub const vpiHexStrVal: i32 = 4;
pub const vpiScalarVal: i32 = 5;
pub const vpiIntVal: i32 = 6;
pub const vpiRealVal: i32 = 7;
pub const vpiStringVal: i32 = 8;
pub const vpiVectorVal: i32 = 9;
pub const vpiObjTypeVal: i32 = 12;
pub const vpiSuppressVal: i32 = 13;

// Time types
pub const vpiScaledRealTime: i32 = 1;
pub const vpiSimTime: i32 = 2;
pub const vpiSuppressTime: i32 = 3;

// Put-value flags
pub const vpiNoDelay: i32 = 1;
pub const vpiInertialDelay: i32 = 2;
pub const vpiForceFlag: i32 = 5;
pub const vpiReleaseFlag: i32 = 6;

// Callback reasons
pub const cbValueChange: i32 = 1;
pub const cbReadWriteSynch: i32 = 6;
pub const cbReadOnlySynch: i32 = 7;
pub const cbNextSimTime: i32 = 8;
pub const cbAfterDelay: i32 = 9;
pub const cbStartOfSimulation: i32 = 11;
pub const cbEndOfSimulation: i32 = 12;

// vpi_control operations
pub const vpiStop: i32 = 66;
pub const vpiFinish: i32 = 67;

// Error levels
pub const vpiNotice: i32 = 1;
pub const vpiWarning: i32 = 2;
pub const vpiError: i32 = 3;
pub const vpiSystem: i32 = 4;
pub const vpiInternal: i32 = 5;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct s_vpi_time {
    pub type_: PLI_INT32,
    pub high: PLI_UINT32,
    pub low: PLI_UINT32,
    pub real: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct s_vpi_vecval {
    pub aval: PLI_UINT32,
    pub bval: PLI_UINT32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub union vpi_value_union {
    pub str_: *mut PLI_BYTE8,
    pub scalar: PLI_INT32,
    pub integer: PLI_INT32,
    pub real: f64,
    pub time: *mut s_vpi_time,
    pub vector: *mut s_vpi_vecval,
    pub strength: *mut c_void,
    pub misc: *mut PLI_BYTE8,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct s_vpi_value {
    pub format: PLI_INT32,
    pub value: vpi_value_union,
}

impl s_vpi_value {
    pub fn new(format: i32) -> s_vpi_value {
        s_vpi_value { format, value: vpi_value_union { integer: 0 } }
    }
}

#[repr(C)]
pub struct s_cb_data {
    pub reason: PLI_INT32,
    pub cb_rtn: Option<unsafe extern "C" fn(*mut s_cb_data) -> PLI_INT32>,
    pub obj: vpiHandle,
    pub time: *mut s_vpi_time,
    pub value: *mut s_vpi_value,
    pub index: PLI_INT32,
    pub user_data: *mut PLI_BYTE8,
}

#[repr(C)]
pub struct s_vpi_error_info {
    pub state: PLI_INT32,
    pub level: PLI_INT32,
    pub message: *mut PLI_BYTE8,
    pub product: *mut PLI_BYTE8,
    pub code: *mut PLI_BYTE8,
    pub file: *mut PLI_BYTE8,
    pub line: PLI_INT32,
}

#[repr(C)]
pub struct s_vpi_vlog_info {
    pub argc: PLI_INT32,
    pub argv: *mut *mut PLI_BYTE8,
    pub product: *mut PLI_BYTE8,
    pub version: *mut PLI_BYTE8,
}

extern "C" {
    pub fn vpi_register_cb(cb_data_p: *mut s_cb_data) -> vpiHandle;
    pub fn vpi_remove_cb(cb_obj: vpiHandle) -> PLI_INT32;
    pub fn vpi_handle_by_name(name: *const PLI_BYTE8, scope: vpiHandle) -> vpiHandle;
    pub fn vpi_handle_by_index(object: vpiHandle, indx: PLI_INT32) -> vpiHandle;
    pub fn vpi_handle(type_: PLI_INT32, refHandle: vpiHandle) -> vpiHandle;
    pub fn vpi_iterate(type_: PLI_INT32, refHandle: vpiHandle) -> vpiHandle;
    pub fn vpi_scan(iterator: vpiHandle) -> vpiHandle;
    pub fn vpi_get(property: PLI_INT32, object: vpiHandle) -> PLI_INT32;
    pub fn vpi_get_str(property: PLI_INT32, object: vpiHandle) -> *mut PLI_BYTE8;
    pub fn vpi_get_value(expr: vpiHandle, value_p: *mut s_vpi_value);
    pub fn vpi_put_value(
        object: vpiHandle,
        value_p: *mut s_vpi_value,
        time_p: *mut s_vpi_time,
        flags: PLI_INT32,
    ) -> vpiHandle;
    pub fn vpi_get_time(object: vpiHandle, time_p: *mut s_vpi_time);
    pub fn vpi_free_object(object: vpiHandle) -> PLI_INT32;
    pub fn vpi_chk_error(error_info_p: *mut s_vpi_error_info) -> PLI_INT32;
    pub fn vpi_get_vlog_info(vlog_info_p: *mut s_vpi_vlog_info) -> PLI_INT32;
    pub fn vpi_control(operation: PLI_INT32, ...) -> PLI_INT32;
}

/// Convert a simulator-owned C string to an owned `String` (empty if NULL).
///
/// # Safety
/// `p` must be NULL or point to a NUL-terminated string.
pub unsafe fn cstr(p: *const c_char) -> String {
    if p.is_null() {
        String::new()
    } else {
        std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
    }
}
