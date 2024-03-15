//! Rust library for interfacing with illumos kernel statistics, `libkstat`.
//!
//! The illumos `kstat` system is a kernel module for exporting data about the
//! system to user processes. Users create a control handle to the system with
//! [`Ctl::new`], which gives them access to the statistics exported by their
//! system.
//!
//! Individual statistics are represented by the [`Kstat`] type, which includes
//! information about the type of data, when it was created or last updated, and
//! the actual data itself. The `Ctl` handle maintains a linked list of `Kstat`
//! objects, which users may walk with the [`Ctl::iter`] method.
//!
//! Each kstat is identified by a module, an instance number, and a name. In
//! addition, the data may be of several different types, such as name/value
//! pairs or interrupt statistics. These types are captured by the [`Data`]
//! enum, which can be read and returned by using the [`Ctl::read`] method.

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::cmp::{Ord, Ordering, PartialOrd};
use std::convert::TryFrom;
use std::ffi::CStr;
use std::marker::PhantomData;
use std::mem::size_of;
use std::os::raw::c_char;

use thiserror::Error;

use libkstat_sys as sys;

/// Kinds of errors returned by the library.
#[derive(Debug, Error)]
pub enum Error {
    /// An attempt to convert a byte-string to a Rust string failed.
    #[error("The byte-string is not a valid Rust string")]
    InvalidString,

    /// Encountered an invalid kstat type.
    #[error("Kstat type {0} is invalid")]
    InvalidType(u8),

    /// Encountered an invalid named kstat data type.
    #[error("The named kstat data type {0} is invalid")]
    InvalidNamedType(u8),

    /// Encountered a null pointer or empty data.
    #[error("A null pointer or empty kstat was encountered")]
    NullData,

    /// Error bubbled up from operating on `libkstat`.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// `Ctl` is a handle to the kstat library.
///
/// Users instantiate a control handle and access the kstat's it contains, for
/// example via the [`Ctl::iter`] method.
#[derive(Debug)]
pub struct Ctl {
    ctl: *mut sys::kstat_ctl_t,
}

/// The `Ctl` wraps a raw pointer allocated by the `libkstat(3KSTAT)` library.
/// This itself isn't thread-safe, but doesn't refer to any thread-local state.
/// So it's safe to send across threads.
unsafe impl Send for Ctl {}

impl Ctl {
    /// Create a new `Ctl`.
    pub fn new() -> Result<Self, Error> {
        let ctl = unsafe { sys::kstat_open() };
        if ctl.is_null() {
            Err(std::io::Error::last_os_error().into())
        } else {
            Ok(Ctl { ctl })
        }
    }

    /// Synchronize this `Ctl` with the kernel's view of the data.
    ///
    /// A `Ctl` is really a snapshot of the kernel's internal list of kstats.
    /// This method consumes and updates a control object, bringing it into sync
    /// with the kernel's copy.
    pub fn update(self) -> Result<Self, Error> {
        let kid = unsafe { sys::kstat_chain_update(self.ctl) };
        if kid == -1 {
            Err(std::io::Error::last_os_error().into())
        } else {
            Ok(self)
        }
    }

    /// Return an iterator over the [`Kstat`]s in `self`.
    ///
    /// Note that this will only return `Kstat`s which are successfully read.
    /// For example, it will ignore those with non-UTF-8 names.
    pub fn iter(&self) -> Iter<'_> {
        Iter { kstat: unsafe { (*self.ctl).kc_chain }, _d: PhantomData }
    }

    /// Read a [`Kstat`], returning the data for it.
    pub fn read<'a>(&self, kstat: &mut Kstat<'a>) -> Result<Data<'a>, Error> {
        kstat.read(self.ctl)?;
        kstat.data()
    }

    /// Find [`Kstat`]s by module, instance, and/or name.
    ///
    /// If a field is `None`, any matching `Kstat` is returned.
    pub fn filter<'a>(
        &'a self,
        module: Option<&'a str>,
        instance: Option<i32>,
        name: Option<&'a str>,
    ) -> impl Iterator<Item = Kstat<'a>> {
        self.iter().filter(move |kstat| {
            module.map(|m| m == kstat.ks_module).unwrap_or(true)
                || instance.map(|i| i == kstat.ks_instance).unwrap_or(true)
                || name.map(|n| n == kstat.ks_name).unwrap_or(true)
        })
    }
}

impl Drop for Ctl {
    fn drop(&mut self) {
        unsafe {
            sys::kstat_close(self.ctl);
        }
    }
}

#[derive(Debug)]
pub struct Iter<'a> {
    kstat: *mut sys::kstat_t,
    _d: PhantomData<&'a ()>,
}

impl<'a> Iterator for Iter<'a> {
    type Item = Kstat<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(ks) = unsafe { self.kstat.as_ref() } {
                self.kstat = unsafe { *self.kstat }.ks_next;
                if let Ok(ks) = Kstat::try_from(ks) {
                    break Some(ks);
                }
                // continue to next kstat
            } else {
                break None;
            }
        }
    }
}

unsafe impl<'a> Send for Iter<'a> {}

/// `Kstat` represents a single kernel statistic.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Kstat<'a> {
    /// The creation time of the stat, in nanoseconds.
    pub ks_crtime: i64,
    /// The time of the last update, in nanoseconds.
    pub ks_snaptime: i64,
    /// The module of the kstat.
    pub ks_module: &'a str,
    /// The instance of the kstat.
    pub ks_instance: i32,
    /// The name of the kstat.
    pub ks_name: &'a str,
    /// The type of the kstat.
    pub ks_type: Type,
    /// The class of the kstat.
    pub ks_class: &'a str,
    ks: *mut sys::kstat_t,
}

impl<'a> PartialOrd for Kstat<'a> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(std::cmp::Ord::cmp(self, other))
    }
}

impl<'a> Ord for Kstat<'a> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.ks_class
            .cmp(other.ks_class)
            .then_with(|| self.ks_module.cmp(other.ks_module))
            .then_with(|| self.ks_instance.cmp(&other.ks_instance))
            .then_with(|| self.ks_name.cmp(other.ks_name))
            .then_with(|| self.ks_class.cmp(other.ks_name))
    }
}

unsafe impl<'a> Send for Kstat<'a> {}

impl<'a> Kstat<'a> {
    fn read(&mut self, ctl: *mut sys::kstat_ctl_t) -> Result<(), Error> {
        if unsafe { sys::kstat_read(ctl, self.ks, std::ptr::null_mut()) } == -1
        {
            Err(std::io::Error::last_os_error().into())
        } else {
            self.ks_snaptime = unsafe { (*self.ks).ks_snaptime };
            Ok(())
        }
    }

    fn data(&self) -> Result<Data<'a>, Error> {
        let ks = unsafe { self.ks.as_ref() }.ok_or_else(|| Error::NullData)?;
        match self.ks_type {
            Type::Raw => {
                if ks.ks_ndata == 0 {
                    Ok(Data::Raw(Vec::new()))
                } else {
                    let item_size = ks.ks_data_size / ks.ks_ndata as usize;
                    let mut start = ks.ks_data as *const u8;
                    let mut out = Vec::with_capacity(ks.ks_ndata as usize);
                    for _ in 0..ks.ks_ndata {
                        out.push(unsafe {
                            std::slice::from_raw_parts(start, item_size)
                        });
                        start = unsafe { start.add(item_size) };
                    }
                    Ok(Data::Raw(out))
                }
            }
            Type::Named => {
                let reported_count = ks.ks_ndata as usize;
                let actual_count =
                    ks.ks_data_size / size_of::<sys::kstat_named_t>();
                let count = reported_count.min(actual_count);
                let data_ents = unsafe {
                    std::slice::from_raw_parts(ks.ks_data as *const _, count)
                };

                Ok(Data::Named(
                    data_ents
                        .iter()
                        .map(Named::try_from)
                        .collect::<Result<_, _>>()?,
                ))
            }
            Type::Intr => {
                assert!(ks.ks_ndata == 1);
                assert!(ks.ks_data_size == size_of::<sys::kstat_intr_t>());

                let ks_intr = unsafe {
                    (ks.ks_data as *const sys::kstat_intr_t).as_ref()
                }
                .unwrap();
                Ok(Data::Intr(Intr::from(ks_intr)))
            }
            Type::Io => {
                assert!(ks.ks_ndata == 1);
                assert!(ks.ks_data_size == size_of::<sys::kstat_io_t>());

                let ks_io =
                    unsafe { (ks.ks_data as *const sys::kstat_io_t).as_ref() }
                        .unwrap();
                Ok(Data::Io(Io::from(ks_io)))
            }
            Type::Timer => {
                assert!(
                    ks.ks_data_size
                        == (ks.ks_ndata as usize
                            * size_of::<sys::kstat_timer_t>())
                );
                let ks_timers = unsafe {
                    std::slice::from_raw_parts(
                        ks.ks_data as *const sys::kstat_timer_t,
                        ks.ks_ndata as _,
                    )
                };

                Ok(Data::Timer(
                    ks_timers
                        .iter()
                        .map(Timer::try_from)
                        .collect::<Result<_, _>>()?,
                ))
            }
        }
    }
}

impl<'a> TryFrom<&'a sys::kstat_t> for Kstat<'a> {
    type Error = Error;
    fn try_from(k: &'a sys::kstat_t) -> Result<Self, Self::Error> {
        Ok(Kstat {
            ks_crtime: k.ks_crtime,
            ks_snaptime: k.ks_snaptime,
            ks_module: kstat_str_parse(&k.ks_module)?,
            ks_instance: k.ks_instance,
            ks_name: kstat_str_parse(&k.ks_name)?,
            ks_type: Type::try_from(k.ks_type)?,
            ks_class: kstat_str_parse(&k.ks_name)?,
            ks: k as *const _ as *mut _,
        })
    }
}

/// The type of a kstat.
#[derive(Debug, Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
pub enum Type {
    Raw,
    Named,
    Intr,
    Io,
    Timer,
}

impl TryFrom<u8> for Type {
    type Error = Error;
    fn try_from(t: u8) -> Result<Self, Self::Error> {
        match t {
            sys::KSTAT_TYPE_RAW => Ok(Type::Raw),
            sys::KSTAT_TYPE_NAMED => Ok(Type::Named),
            sys::KSTAT_TYPE_INTR => Ok(Type::Intr),
            sys::KSTAT_TYPE_IO => Ok(Type::Io),
            sys::KSTAT_TYPE_TIMER => Ok(Type::Timer),
            other => Err(Self::Error::InvalidType(other)),
        }
    }
}

/// The data type of a single name/value pair of a named kstat.
#[derive(Debug, Copy, Clone, PartialEq)]
pub enum NamedType {
    Char,
    Int32,
    UInt32,
    Int64,
    UInt64,
    String,
}

impl TryFrom<u8> for NamedType {
    type Error = Error;
    fn try_from(t: u8) -> Result<Self, Self::Error> {
        match t {
            sys::KSTAT_DATA_CHAR => Ok(NamedType::Char),
            sys::KSTAT_DATA_INT32 => Ok(NamedType::Int32),
            sys::KSTAT_DATA_UINT32 => Ok(NamedType::UInt32),
            sys::KSTAT_DATA_INT64 => Ok(NamedType::Int64),
            sys::KSTAT_DATA_UINT64 => Ok(NamedType::UInt64),
            sys::KSTAT_DATA_STRING => Ok(NamedType::String),
            other => Err(Self::Error::InvalidNamedType(other)),
        }
    }
}

/// Data from a single kstat.
#[derive(Clone, Debug)]
pub enum Data<'a> {
    Raw(Vec<&'a [u8]>),
    Named(Vec<Named<'a>>),
    Intr(Intr),
    Io(Io),
    Timer(Vec<Timer<'a>>),
    Null,
}

/// An I/O kernel statistic
#[derive(Debug, Clone, Copy)]
pub struct Io {
    pub nread: u64,
    pub nwritten: u64,
    pub reads: u32,
    pub writes: u32,
    pub wtime: i64,
    pub wlentime: i64,
    pub wlastupdate: i64,
    pub rtime: i64,
    pub rlentime: i64,
    pub rlastupdate: i64,
    pub wcnt: u32,
    pub rcnt: u32,
}

impl From<&sys::kstat_io_t> for Io {
    fn from(k: &sys::kstat_io_t) -> Self {
        Io {
            nread: k.nread,
            nwritten: k.nwritten,
            reads: k.reads,
            writes: k.writes,
            wtime: k.wtime,
            wlentime: k.wlentime,
            wlastupdate: k.wlastupdate,
            rtime: k.rtime,
            rlentime: k.rlentime,
            rlastupdate: k.rlastupdate,
            wcnt: k.wcnt,
            rcnt: k.rcnt,
        }
    }
}

/// A timer kernel statistic.
#[derive(Debug, Copy, Clone)]
pub struct Timer<'a> {
    pub name: &'a str,
    pub num_events: usize,
    pub elapsed_time: i64,
    pub min_time: i64,
    pub max_time: i64,
    pub start_time: i64,
    pub stop_time: i64,
}

impl<'a> TryFrom<&'a sys::kstat_timer_t> for Timer<'a> {
    type Error = Error;
    fn try_from(k: &'a sys::kstat_timer_t) -> Result<Self, Self::Error> {
        Ok(Self {
            name: kstat_str_parse(&k.name)?,
            num_events: k.num_events as _,
            elapsed_time: k.elapsed_time,
            min_time: k.min_time,
            max_time: k.max_time,
            start_time: k.start_time,
            stop_time: k.stop_time,
        })
    }
}

/// Interrupt kernel statistic.
#[derive(Debug, Copy, Clone)]
pub struct Intr {
    pub hard: u32,
    pub soft: u32,
    pub watchdog: u32,
    pub spurious: u32,
    pub multisvc: u32,
}

impl From<&sys::kstat_intr_t> for Intr {
    fn from(k: &sys::kstat_intr_t) -> Self {
        Self {
            hard: k.intr_hard,
            soft: k.intr_soft,
            watchdog: k.intr_watchdog,
            spurious: k.intr_spurious,
            multisvc: k.intr_multisvc,
        }
    }
}

/// A name/value data element from a named kernel statistic.
#[derive(Clone, Debug)]
pub struct Named<'a> {
    pub name: &'a str,
    pub value: NamedData<'a>,
}

impl<'a> Named<'a> {
    /// Return the data type of a named kernel statistic.
    pub fn data_type(&self) -> NamedType {
        self.value.data_type()
    }
}

/// The value part of a name-value kernel statistic.
#[derive(Clone, Debug)]
pub enum NamedData<'a> {
    Char(&'a [u8]),
    Int32(i32),
    UInt32(u32),
    Int64(i64),
    UInt64(u64),
    String(&'a str),
}

impl<'a> NamedData<'a> {
    /// Return the data type of a named kernel statistic.
    pub fn data_type(&self) -> NamedType {
        match self {
            NamedData::Char(_) => NamedType::Char,
            NamedData::Int32(_) => NamedType::Int32,
            NamedData::UInt32(_) => NamedType::UInt32,
            NamedData::Int64(_) => NamedType::Int64,
            NamedData::UInt64(_) => NamedType::UInt64,
            NamedData::String(_) => NamedType::String,
        }
    }
}

impl<'a> TryFrom<&'a sys::kstat_named_t> for Named<'a> {
    type Error = Error;
    fn try_from(k: &'a sys::kstat_named_t) -> Result<Self, Self::Error> {
        let name = kstat_str_parse(&k.name)?;
        match NamedType::try_from(k.data_type)? {
            NamedType::Char => {
                let slice = unsafe {
                    std::slice::from_raw_parts(
                        k.value.charc.as_ptr(),
                        k.value.charc.len(),
                    )
                };
                Ok(Named { name, value: NamedData::Char(slice) })
            }
            NamedType::Int32 => Ok(Named {
                name,
                value: NamedData::Int32(unsafe { k.value.i32 }),
            }),
            NamedType::UInt32 => Ok(Named {
                name,
                value: NamedData::UInt32(unsafe { k.value.ui32 }),
            }),
            NamedType::Int64 => Ok(Named {
                name,
                value: NamedData::Int64(unsafe { k.value.i64 }),
            }),

            NamedType::UInt64 => Ok(Named {
                name,
                value: NamedData::UInt64(unsafe { k.value.ui64 }),
            }),
            NamedType::String => {
                let data_cstr = unsafe { CStr::from_ptr(k.value.str.addr) };
                let data_str =
                    data_cstr.to_str().map_err(|_| Error::InvalidString)?;

                Ok(Named { name, value: NamedData::String(data_str) })
            }
        }
    }
}

pub(crate) fn kstat_str_parse(
    s: &[c_char; sys::KSTAT_STRLEN],
) -> Result<&str, Error> {
    unsafe { CStr::from_ptr(s.as_ptr() as *const _) }
        .to_str()
        .map_err(|_| Error::InvalidString)
}

#[cfg(test)]
mod test {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn basic_test() {
        let ctl = Ctl::new().expect("Failed to create kstat control");
        for mut kstat in ctl.iter() {
            match ctl.read(&mut kstat) {
                Ok(_) => {}
                Err(e) => {
                    println!("{}", e);
                }
            }
        }
    }

    #[test]
    fn compare_with_kstat_cli() {
        let ctl = Ctl::new().expect("Failed to create kstat control");
        let mut kstat = ctl
            .filter(Some("cpu_info"), Some(0), Some("cpu_info0"))
            .next()
            .expect("Failed to find kstat cpu_info:0:cpu_info0");
        if let Data::Named(data) =
            ctl.read(&mut kstat).expect("Failed to read kstat")
        {
            let mut items = BTreeMap::new();
            for item in data.iter() {
                items.insert(item.name, item);
            }
            let out = subprocess::Exec::cmd("/usr/bin/kstat")
                .arg("-p")
                .arg("cpu_info:0:cpu_info0:")
                .stdout(subprocess::Redirection::Pipe)
                .capture()
                .expect("Failed to run /usr/bin/kstat");
            let kstat_items: BTreeMap<_, _> = String::from_utf8(out.stdout)
                .expect("Non UTF-8 output from kstat")
                .lines()
                .filter_map(|line| {
                    let parts = line.trim().split('\t').collect::<Vec<_>>();
                    assert_eq!(
                        parts.len(),
                        2,
                        "Lines from kstat should be 2 tab-separated items, found {:#?}",
                        parts
                    );
                    let (id, value) = (parts[0], parts[1]);
                    if id.ends_with("crtime") {
                        let crtime: f64 = value.parse().expect("Expected a crtime in nanoseconds");
                        let crtime = (crtime * 1e9) as i64;
                        assert!(
                            (crtime - kstat.ks_crtime) < 5 || (kstat.ks_crtime - crtime) < 5,
                            "Expected nearly equal crtimes"
                        );
                        // Don't push this value
                        None
                    } else if id.ends_with("snaptime") {
                        let snaptime: f64 =
                            value.parse().expect("Expected a snaptime in nanoseconds");
                        let snaptime = (snaptime * 1e9) as i64;
                        assert!(
                            (snaptime - kstat.ks_snaptime) < 5
                                || (kstat.ks_snaptime - snaptime) < 5,
                            "Expected nearly equal snaptimes"
                        );
                        // Don't push this value
                        None
                    } else if id.ends_with("class") {
                        // Don't push this value
                        None
                    } else {
                        Some((id.to_string(), value.to_string()))
                    }
                })
                .collect();
            assert_eq!(
                items.len(),
                kstat_items.len(),
                "Expected the same number of items from /usr/bin/kstat:\n{:#?}\n{:#?}",
                items,
                kstat_items
            );
            const SKIPPED_STATS: &[&'static str] =
                &["current_clock_Hz", "current_cstate"];
            for (key, value) in kstat_items.iter() {
                let name =
                    key.split(':').last().expect("Expected to split on ':'");
                if SKIPPED_STATS.contains(&name) {
                    println!(
                        "Skipping stat '{}', not stable enough for testing",
                        name
                    );
                    continue;
                }
                let item = items.get(name).expect(&format!(
                    "Expected a name/value pair with name '{}'",
                    name
                ));
                println!("key: {:#?}\nvalue: {:#?}", key, value);
                println!("item: {:#?}", item);
                match item.value {
                    NamedData::Char(slice) => {
                        for (sl, by) in
                            slice.iter().zip(value.as_bytes().iter())
                        {
                            if by == &0 {
                                break;
                            }
                            assert_eq!(
                                sl, by,
                                "Expected equal bytes, found {} and {}",
                                sl, by
                            );
                        }
                    }
                    NamedData::Int32(i) => {
                        assert_eq!(i, value.parse().unwrap())
                    }
                    NamedData::UInt32(u) => {
                        assert_eq!(u, value.parse().unwrap())
                    }
                    NamedData::Int64(i) => {
                        assert_eq!(i, value.parse().unwrap())
                    }
                    NamedData::UInt64(u) => {
                        assert_eq!(u, value.parse().unwrap())
                    }
                    NamedData::String(s) => assert_eq!(s, value),
                }
            }
        }
    }
}
