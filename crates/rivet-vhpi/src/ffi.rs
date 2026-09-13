//! Minimal hand-written bindings to IEEE 1076-2019 VHPI (`vhpi_user.h`).
//!
//! Only the subset Rivet uses. Symbols resolve at load time against the
//! simulator (NVC's `--load`, Questa's `-foreign`, Riviera's `-loadvhpi`).
//! Constant values are taken from NVC's `vhpi_user.h`, which follows the
//! standard's numbering.

#![allow(non_camel_case_types, non_snake_case, non_upper_case_globals, dead_code)]

use std::os::raw::{c_char, c_int, c_void};

pub type vhpiHandleT = *mut u32;
pub type vhpiEnumT = u32;
pub type vhpiSmallEnumT = u8;
pub type vhpiIntT = i32;
pub type vhpiLongIntT = i64;
pub type vhpiCharT = u8;
pub type vhpiRealT = f64;

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct vhpiPhysT {
    pub high: i32,
    pub low: u32,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct vhpiTimeT {
    pub high: i32,
    pub low: u32,
}

impl vhpiPhysT {
    pub fn to_i64(self) -> i64 {
        ((self.high as i64) << 32) | (self.low as i64)
    }
}

impl vhpiTimeT {
    pub fn to_u64(self) -> u64 {
        ((self.high as u64) << 32) | (self.low as u64)
    }
    pub fn from_u64(v: u64) -> vhpiTimeT {
        vhpiTimeT { high: (v >> 32) as i32, low: (v & 0xffff_ffff) as u32 }
    }
}

#[repr(C)]
#[derive(Copy, Clone)]
pub union vhpiValueUnion {
    pub enumv: vhpiEnumT,
    pub enumvs: *mut vhpiEnumT,
    pub smallenumv: vhpiSmallEnumT,
    pub smallenumvs: *mut vhpiSmallEnumT,
    pub intg: vhpiIntT,
    pub intgs: *mut vhpiIntT,
    pub longintg: vhpiLongIntT,
    pub real: vhpiRealT,
    pub phys: vhpiPhysT,
    pub time: vhpiTimeT,
    pub ch: vhpiCharT,
    pub str_: *mut vhpiCharT,
    pub ptr: *mut c_void,
}

#[repr(C)]
pub struct vhpiValueT {
    pub format: c_int,
    pub bufSize: usize,
    pub numElems: i32,
    pub unit: vhpiPhysT,
    pub value: vhpiValueUnion,
}

impl Default for vhpiValueT {
    fn default() -> Self {
        vhpiValueT {
            format: vhpiObjTypeVal,
            bufSize: 0,
            numElems: 0,
            unit: vhpiPhysT::default(),
            value: vhpiValueUnion { ptr: std::ptr::null_mut() },
        }
    }
}

#[repr(C)]
pub struct vhpiCbDataT {
    pub reason: i32,
    pub cb_rtn: Option<unsafe extern "C" fn(*const vhpiCbDataT)>,
    pub obj: vhpiHandleT,
    pub time: *mut vhpiTimeT,
    pub value: *mut vhpiValueT,
    pub user_data: *mut c_void,
}

#[repr(C)]
pub struct vhpiErrorInfoT {
    pub severity: c_int,
    pub message: *mut c_char,
    pub str_: *mut c_char,
    pub file: *mut c_char,
    pub line: i32,
}

// Value formats
pub const vhpiBinStrVal: c_int = 1;
pub const vhpiEnumVal: c_int = 5;
pub const vhpiIntVal: c_int = 6;
pub const vhpiLogicVal: c_int = 7;
pub const vhpiRealVal: c_int = 8;
pub const vhpiStrVal: c_int = 9;
pub const vhpiCharVal: c_int = 10;
pub const vhpiObjTypeVal: c_int = 13;
pub const vhpiEnumVecVal: c_int = 15;
pub const vhpiIntVecVal: c_int = 16;
pub const vhpiLogicVecVal: c_int = 17;
pub const vhpiSmallEnumVal: c_int = 23;
pub const vhpiSmallEnumVecVal: c_int = 24;
pub const vhpiLongIntVal: c_int = 25;

// Class kinds (vhpiClassKindT)
pub const vhpiArrayTypeDeclK: i32 = 1009;
pub const vhpiBlockStmtK: i32 = 1017;
pub const vhpiCompInstStmtK: i32 = 1024;
pub const vhpiConstDeclK: i32 = 1028;
pub const vhpiEnumTypeDeclK: i32 = 1041;
pub const vhpiForGenerateK: i32 = 1048;
pub const vhpiFloatTypeDeclK: i32 = 1047;
pub const vhpiGenericDeclK: i32 = 1053;
pub const vhpiIfGenerateK: i32 = 1056;
pub const vhpiIndexedNameK: i32 = 1059;
pub const vhpiIntTypeDeclK: i32 = 1062;
pub const vhpiPackInstK: i32 = 1074;
pub const vhpiPhysTypeDeclK: i32 = 1078;
pub const vhpiPortDeclK: i32 = 1079;
pub const vhpiProcessStmtK: i32 = 1082;
pub const vhpiRecordTypeDeclK: i32 = 1087;
pub const vhpiRootInstK: i32 = 1090;
pub const vhpiSelectedNameK: i32 = 1093;
pub const vhpiSigDeclK: i32 = 1094;
pub const vhpiSigParamDeclK: i32 = 1095;
pub const vhpiSubpBodyK: i32 = 1100;
pub const vhpiSubtypeDeclK: i32 = 1101;
pub const vhpiVarDeclK: i32 = 1110;

// One-to-one relations
pub const vhpiBaseType: c_int = 1306;
pub const vhpiElemType: c_int = 1380;
pub const vhpiDesignUnit: c_int = 1321;
pub const vhpiRootInst: c_int = 1361;
pub const vhpiValExpr: c_int = 1378;
/// Deprecated in the 2019 standard, still the only working answer on some
/// tools when `vhpiBaseType` returns nothing (cocotb VhpiImpl.cpp:301).
pub const vhpiSubtype_DEPRECATED: c_int = 1367;
pub const vhpiElemSubtype_DEPRECATED: c_int = 1323;

// One-to-many relations
pub const vhpiBlockStmts: c_int = 1506;
pub const vhpiCompInstStmts: c_int = 1510;
pub const vhpiConstDecls: c_int = 1515;
pub const vhpiConstraints: c_int = 1516;
pub const vhpiDecls: c_int = 1519;
pub const vhpiEnumLiterals: c_int = 1527;
pub const vhpiGenericDecls: c_int = 1530;
pub const vhpiIndexedNames: c_int = 1532;
pub const vhpiInternalRegions: c_int = 1533;
pub const vhpiPortDecls: c_int = 1539;
pub const vhpiSelectedNames: c_int = 1542;
pub const vhpiSigDecls: c_int = 1546;
pub const vhpiStmts: c_int = 1551;
pub const vhpiVarDecls: c_int = 1556;

// Integer properties
pub const vhpiIsLocalP: c_int = 1023;
pub const vhpiIsCompositeP: c_int = 1014;
pub const vhpiIsScalarP: c_int = 1033;
pub const vhpiIsUnconstrainedP: c_int = 1038;
pub const vhpiIsUpP: c_int = 1040;
pub const vhpiKindP: c_int = 1043;
pub const vhpiLeftBoundP: c_int = 1044;
pub const vhpiModeP: c_int = 1049;
pub const vhpiNumDimensionsP: c_int = 1050;
pub const vhpiNumLiteralsP: c_int = 1053;
pub const vhpiRightBoundP: c_int = 1063;
pub const vhpiSizeP: c_int = 1065;
pub const vhpiStaticnessP: c_int = 1068;

// String properties
pub const vhpiCaseNameP: c_int = 1301;
pub const vhpiDefNameP: c_int = 1303;
pub const vhpiFullCaseNameP: c_int = 1305;
pub const vhpiFullNameP: c_int = 1306;
pub const vhpiKindStrP: c_int = 1307;
pub const vhpiNameP: c_int = 1313;
pub const vhpiStrValP: c_int = 1315;
pub const vhpiToolVersionP: c_int = 1316;
pub const vhpiUnitNameP: c_int = 1317;

// Physical properties
pub const vhpiResolutionLimitP: c_int = 1657;

// Put-value modes (vhpiPutValueModeT, numbered from zero)
pub const vhpiDeposit: c_int = 0;
pub const vhpiDepositPropagate: c_int = 1;
pub const vhpiForce: c_int = 2;
pub const vhpiForcePropagate: c_int = 3;
pub const vhpiRelease: c_int = 4;
pub const vhpiSizeConstraint: c_int = 5;

// Simulation control
pub const vhpiStop: c_int = 0;
pub const vhpiFinish: c_int = 1;
pub const vhpiReset: c_int = 2;

// Callback reasons
pub const vhpiCbValueChange: i32 = 1001;
pub const vhpiCbAfterDelay: i32 = 1010;
pub const vhpiCbRepAfterDelay: i32 = 1011;
pub const vhpiCbNextTimeStep: i32 = 1012;
pub const vhpiCbRepNextTimeStep: i32 = 1013;
pub const vhpiCbStartOfNextCycle: i32 = 1014;
pub const vhpiCbEndOfProcesses: i32 = 1018;
pub const vhpiCbRepEndOfProcesses: i32 = 1019;
pub const vhpiCbLastKnownDeltaCycle: i32 = 1020;
pub const vhpiCbRepLastKnownDeltaCycle: i32 = 1021;
pub const vhpiCbEndOfTimeStep: i32 = 1024;
pub const vhpiCbRepEndOfTimeStep: i32 = 1025;
pub const vhpiCbStartOfSimulation: i32 = 1034;
pub const vhpiCbEndOfSimulation: i32 = 1035;

// Callback flags
pub const vhpiReturnCb: i32 = 0x0000_0001;
pub const vhpiDisableCb: i32 = 0x0000_0010;

extern "C" {
    pub fn vhpi_register_cb(cb_data_p: *mut vhpiCbDataT, flags: i32) -> vhpiHandleT;
    pub fn vhpi_remove_cb(cb_obj: vhpiHandleT) -> c_int;
    pub fn vhpi_handle_by_name(name: *const c_char, scope: vhpiHandleT) -> vhpiHandleT;
    pub fn vhpi_handle_by_index(itRel: c_int, parent: vhpiHandleT, index: i32) -> vhpiHandleT;
    pub fn vhpi_handle(type_: c_int, referenceHandle: vhpiHandleT) -> vhpiHandleT;
    pub fn vhpi_iterator(type_: c_int, referenceHandle: vhpiHandleT) -> vhpiHandleT;
    pub fn vhpi_scan(iterator: vhpiHandleT) -> vhpiHandleT;
    pub fn vhpi_get(property: c_int, object: vhpiHandleT) -> vhpiIntT;
    pub fn vhpi_get_str(property: c_int, object: vhpiHandleT) -> *const vhpiCharT;
    pub fn vhpi_get_real(property: c_int, object: vhpiHandleT) -> vhpiRealT;
    pub fn vhpi_get_phys(property: c_int, object: vhpiHandleT) -> vhpiPhysT;
    pub fn vhpi_get_value(expr: vhpiHandleT, value_p: *mut vhpiValueT) -> c_int;
    pub fn vhpi_put_value(object: vhpiHandleT, value_p: *mut vhpiValueT, mode: c_int) -> c_int;
    pub fn vhpi_get_time(time_p: *mut vhpiTimeT, cycles: *mut u64);
    pub fn vhpi_control(command: c_int, ...) -> c_int;
    pub fn vhpi_check_error(error_info_p: *mut vhpiErrorInfoT) -> c_int;
    pub fn vhpi_release_handle(object: vhpiHandleT) -> c_int;
    pub fn vhpi_printf(format: *const c_char, ...) -> c_int;
}

/// Read a simulator-owned C string.
///
/// # Safety
/// `p` must be NUL-terminated or null.
pub unsafe fn cstr(p: *const vhpiCharT) -> String {
    if p.is_null() {
        return String::new();
    }
    std::ffi::CStr::from_ptr(p as *const c_char).to_string_lossy().into_owned()
}
