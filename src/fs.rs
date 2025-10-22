use std::ffi::CString;
use std::mem::MaybeUninit;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use lazy_static::lazy_static;

const DIR_RBYTES_ATTR: &str = "ceph.dir.rbytes";
const DIR_RCTIME_ATTR: &str = "ceph.dir.rctime";
const DIR_RENTRIES_ATTR: &str = "ceph.dir.rentries";

lazy_static! {
    static ref DIR_RBYTES_ATTR_C: CString = CString::new(DIR_RBYTES_ATTR).unwrap();
    static ref DIR_RCTIME_ATTR_C: CString = CString::new(DIR_RCTIME_ATTR).unwrap();
    static ref DIR_RENTRIES_ATTR_C: CString = CString::new(DIR_RENTRIES_ATTR).unwrap();
}

lazy_static! {
    static ref NAME_CACHE: std::sync::Mutex<std::collections::HashMap<u32, String>> =
        std::sync::Mutex::new(std::collections::HashMap::new());
}

#[derive(Debug, Clone, Copy)]
pub struct FSType {
    inner: i64,
}

impl FSType {
    pub fn is_ceph(self: FSType) -> bool {
        // TODO: what's the "official" f_type?
        self.inner == 0x00c36400 || self.inner == 0x65735546
    }
}

pub fn id_to_name(id: u32) -> Option<String> {
    if let Some(name) = NAME_CACHE.lock().unwrap().get(&id) {
        return Some(name.clone());
    }

    let name = id_to_name_uncached(id)?;

    NAME_CACHE.lock().unwrap().insert(id, name.clone());
    Some(name)
}

fn id_to_name_uncached(id: u32) -> Option<String> {
    let maxsize: usize = {
        let sysconf_value = unsafe { libc::sysconf(libc::_SC_GETPW_R_SIZE_MAX) };
        if sysconf_value <= 0 {
            1024
        } else {
            sysconf_value as usize
        }
    };

    let mut pwd_struct: MaybeUninit<libc::passwd> = MaybeUninit::uninit();
    let mut buf: Vec<libc::c_char> = Vec::with_capacity(maxsize as usize);
    let mut result_ptr_raw: MaybeUninit<*mut libc::passwd> = MaybeUninit::uninit();

    let (result, result_ptr) = unsafe {
        let res = libc::getpwuid_r(
            id,
            pwd_struct.as_mut_ptr(),
            buf.as_mut_ptr() as *mut libc::c_char,
            maxsize as libc::size_t,
            result_ptr_raw.as_mut_ptr(),
        );
        buf.set_len(maxsize as usize);
        (res, result_ptr_raw.assume_init())
    };

    if result != 0 || result_ptr.is_null() {
        return None;
    }

    let name = unsafe {
        let pwd_struct = pwd_struct.assume_init();
        std::ffi::CStr::from_ptr(pwd_struct.pw_name)
    }
    .to_string_lossy()
    .trim()
    .to_owned();
    if name.is_empty() {
        return None;
    }
    Some(name)
}

pub fn get_fs(path: &Path) -> Option<FSType> {
    let c_path = CString::new(path.as_os_str().as_bytes()).ok()?;

    // Create and zero-initialize a statfs buffer
    let mut stat_buf: libc::statfs = unsafe { std::mem::zeroed() };

    // Call statfs and check for error
    let result = unsafe { libc::statfs(c_path.as_ptr(), &mut stat_buf) };

    if result < 0 {
        return None;
    }

    Some(FSType {
        #[allow(clippy::unnecessary_cast)]
        inner: stat_buf.f_type as i64,
    })
}

fn get_xattr(path: &Path, attr: &CString) -> Option<String> {
    // First query the size of the attribute, then fetch it.
    let c_path = CString::new(path.as_os_str().as_bytes()).ok()?;

    let attr_size =
        unsafe { libc::lgetxattr(c_path.as_ptr(), attr.as_ptr(), std::ptr::null_mut(), 0) };

    if attr_size < 0 {
        return None;
    }

    let mut buf = Vec::<u8>::with_capacity(attr_size as usize);
    let attr_size2 = unsafe {
        buf.set_len(attr_size as usize);
        libc::lgetxattr(
            c_path.as_ptr(),
            attr.as_ptr(),
            buf.as_mut_ptr() as *mut libc::c_void,
            attr_size as libc::size_t,
        )
    };
    if attr_size2 < 0 {
        return None;
    }
    Some(String::from_utf8_lossy(&buf).to_string())
}

pub fn get_rentries(path: &Path) -> Option<usize> {
    let rentries = get_xattr(path, &DIR_RENTRIES_ATTR_C)?;
    // convert rentries xattr from string to unsigned
    let rentries = rentries.trim().parse::<usize>().ok()?;
    Some(rentries)
}

pub fn get_rbytes(path: &Path) -> Option<usize> {
    let rbytes = get_xattr(path, &DIR_RBYTES_ATTR_C)?;
    // convert rbytes xattr from string to unsigned
    let rbytes = rbytes.trim().parse::<usize>().ok()?;
    Some(rbytes)
}

pub fn get_rctime(path: &Path) -> Option<usize> {
    let rctime = get_xattr(path, &DIR_RCTIME_ATTR_C)?;
    // convert rctime xattr from string ("seconds.nanos") to unsigned
    let rctime = rctime.trim().split(".").next()?.parse::<usize>().ok()?;
    Some(rctime)
}
