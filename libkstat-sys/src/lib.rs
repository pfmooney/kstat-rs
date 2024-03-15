//! FFI bindings to `libkstat`.

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

#![allow(non_camel_case_types)]

use libc::size_t;

use std::fmt::{self, Debug};
use std::os::raw::{
    c_char, c_int, c_longlong, c_uchar, c_uint, c_ulonglong, c_void,
};

// Kstat types
pub const KSTAT_TYPE_RAW: u8 = 0;
pub const KSTAT_TYPE_NAMED: u8 = 1;
pub const KSTAT_TYPE_INTR: u8 = 2;
pub const KSTAT_TYPE_IO: u8 = 3;
pub const KSTAT_TYPE_TIMER: u8 = 4;

// Named kstat data types
pub const KSTAT_DATA_CHAR: u8 = 0;
pub const KSTAT_DATA_INT32: u8 = 1;
pub const KSTAT_DATA_UINT32: u8 = 2;
pub const KSTAT_DATA_INT64: u8 = 3;
pub const KSTAT_DATA_UINT64: u8 = 4;
pub const KSTAT_DATA_STRING: u8 = 9;

// Length of string array fields
pub const KSTAT_STRLEN: usize = 31;

// Rust FFI equivalent to `libkstat`'s `kstat_ctl_t`.
#[derive(Debug, Copy, Clone)]
#[repr(C)]
pub struct kstat_ctl_t {
    pub kc_chain_id: kid_t,
    pub kc_chain: *mut kstat_t,
    pub kc_kd: c_int,
}

// Type alias for system high-resolution time, expressed in nanoseconds.
pub type hrtime_t = c_longlong;

// Type alias for kstat identifiers.
pub type kid_t = c_int;

// Rust FFI equivalent to `libkstat`'s `kstat_t`.
#[derive(Debug, Copy, Clone)]
#[repr(C)]
pub struct kstat_t {
    pub ks_crtime: hrtime_t,
    pub ks_next: *mut kstat_t,
    pub ks_kid: kid_t,
    pub ks_module: [c_char; KSTAT_STRLEN],
    _ks_resv: c_uchar,
    pub ks_instance: c_int,
    pub ks_name: [c_char; KSTAT_STRLEN],
    pub ks_type: c_uchar,
    pub ks_class: [c_char; KSTAT_STRLEN],
    pub ks_flags: c_char,
    pub ks_data: *mut c_void,
    pub ks_ndata: c_uint,
    pub ks_data_size: size_t,
    pub ks_snaptime: hrtime_t,
}

#[derive(Debug, Copy, Clone)]
#[repr(C)]
pub struct kstat_named_t {
    pub name: [c_char; KSTAT_STRLEN],
    pub data_type: c_uchar,
    pub value: kstat_named_data_u,
}

#[derive(Copy, Clone)]
#[repr(C)]
pub union kstat_named_data_u {
    pub charc: [c_uchar; 16],
    pub str: kstat_named_data_str,
    pub i32: i32,
    pub ui32: u32,
    pub i64: i64,
    pub ui64: u64,
}

impl Debug for kstat_named_data_u {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "kstat_named_data_u(0x{:p})", self as *const _)
    }
}

#[derive(Debug, Copy, Clone)]
#[repr(C)]
pub struct kstat_named_data_str {
    pub addr: *const c_char,
    pub len: u32,
}

#[derive(Debug, Copy, Clone)]
#[repr(C)]
pub struct kstat_intr_t {
    pub intr_hard: u32,
    pub intr_soft: u32,
    pub intr_watchdog: u32,
    pub intr_spurious: u32,
    pub intr_multisvc: u32,
}

#[derive(Debug, Copy, Clone)]
#[repr(C)]
pub struct kstat_timer_t {
    pub name: [c_char; KSTAT_STRLEN],
    _resv: c_uchar,
    pub num_events: c_ulonglong,
    pub elapsed_time: hrtime_t,
    pub min_time: hrtime_t,
    pub max_time: hrtime_t,
    pub start_time: hrtime_t,
    pub stop_time: hrtime_t,
}

#[derive(Debug, Copy, Clone)]
#[repr(C)]
pub struct kstat_io_t {
    pub nread: c_ulonglong,
    pub nwritten: c_ulonglong,
    pub reads: c_uint,
    pub writes: c_uint,
    pub wtime: hrtime_t,
    pub wlentime: hrtime_t,
    pub wlastupdate: hrtime_t,
    pub rtime: hrtime_t,
    pub rlentime: hrtime_t,
    pub rlastupdate: hrtime_t,
    pub wcnt: c_uint,
    pub rcnt: c_uint,
}

#[cfg(any(target_os = "illumos", not(feature = "stubs")))]
mod native_ffi {
    use super::{kid_t, kstat_ctl_t, kstat_t};
    use std::os::raw::c_void;

    #[link(name = "kstat")]
    extern "C" {
        pub fn kstat_open() -> *mut kstat_ctl_t;
        pub fn kstat_close(kc: *mut kstat_ctl_t) -> i32;
        pub fn kstat_read(
            kc: *mut kstat_ctl_t,
            ksp: *mut kstat_t,
            data: *mut c_void,
        ) -> kid_t;
        pub fn kstat_chain_update(kc: *mut kstat_ctl_t) -> kid_t;
    }
}
#[cfg(any(target_os = "illumos", not(feature = "stubs")))]
pub use native_ffi::*;

#[cfg(all(not(target_os = "illumos"), feature = "stubs"))]
mod stub_ffi {
    use super::{kid_t, kstat_ctl_t, kstat_t};
    use std::os::raw::c_void;

    fn errfn() -> ! {
        panic!("libkstat support absent on non-illumos machines")
    }

    pub unsafe fn kstat_open() -> *mut kstat_ctl_t {
        errfn()
    }
    pub unsafe fn kstat_close(_kc: *mut kstat_ctl_t) -> i32 {
        errfn()
    }
    pub unsafe fn kstat_read(
        _kc: *mut kstat_ctl_t,
        _ksp: *mut kstat_t,
        _data: *mut c_void,
    ) -> kid_t {
        errfn()
    }
    pub unsafe fn kstat_chain_update(_kc: *mut kstat_ctl_t) -> kid_t {
        errfn()
    }
}
#[cfg(all(not(target_os = "illumos"), feature = "stubs"))]
pub use stub_ffi::*;
