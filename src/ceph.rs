use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

use lazy_static::lazy_static;
use libc;

const DIR_RENTRIES_ATTR: &str = "ceph.dir.rentries";

lazy_static! {
    static ref DIR_RENTRIES_ATTR_C: CString = CString::new(DIR_RENTRIES_ATTR).unwrap();
}

pub fn get_rentries(path: &PathBuf) -> Option<usize> {
    // First query the size of the attribute, then fetch it.
    let c_path = CString::new(path.as_os_str().as_bytes()).ok()?;

    let attr_size = unsafe {
        libc::lgetxattr(
            c_path.as_ptr(),
            DIR_RENTRIES_ATTR_C.as_ptr(),
            std::ptr::null_mut(),
            0,
        )
    };

    if attr_size < 0 {
        return None;
    }

    let mut buf = Vec::<u8>::with_capacity(attr_size as usize);
    let attr_size2 = unsafe {
        buf.set_len(attr_size as usize);
        libc::lgetxattr(
            c_path.as_ptr(),
            DIR_RENTRIES_ATTR_C.as_ptr(),
            buf.as_mut_ptr() as *mut libc::c_void,
            attr_size as libc::size_t,
        )
    };
    if attr_size2 < 0 {
        return None;
    }

    // rentries is a string, so we need to convert it to a number.
    // let rentries = unsafe { *(buf.as_ptr() as *const usize) };
    // Some(rentries)
    let rentries = String::from_utf8_lossy(&buf);
    let rentries = rentries.trim().parse::<usize>().ok()?;
    Some(rentries)
}
