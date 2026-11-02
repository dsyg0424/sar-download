use std::{
    io,
    path::{Path, PathBuf},
};

#[derive(serde::Serialize)]
pub struct DiskSpace {
    pub volume: String,
    pub available: u64,
}

/// Query the nearest existing directory without creating the requested path.
pub fn probe(path: &Path) -> Result<DiskSpace, String> {
    if !path.is_absolute() {
        return Err("保存目录必须是绝对路径".into());
    }
    let resolved = resolve_existing(path)?;
    let mut existing = resolved.as_path();
    while !existing.exists() {
        existing = existing.parent().ok_or("目录不可用")?;
    }
    if !existing.is_dir() {
        return Err("保存位置不是文件夹".into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::GetVolumePathNameW;
        let wide: Vec<u16> = existing.as_os_str().encode_wide().chain(Some(0)).collect();
        let available = available_bytes(existing)?;
        let mut volume = vec![0u16; 32768];
        unsafe {
            if GetVolumePathNameW(wide.as_ptr(), volume.as_mut_ptr(), volume.len() as u32) == 0 {
                return Err(format!("无法读取磁盘空间：{}", io::Error::last_os_error()));
            }
        }
        let end = volume.iter().position(|v| *v == 0).unwrap_or(volume.len());
        Ok(DiskSpace {
            volume: String::from_utf16_lossy(&volume[..end]).to_lowercase(),
            available,
        })
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let device = std::fs::metadata(existing)
            .map_err(|e| e.to_string())?
            .dev();
        Ok(DiskSpace {
            volume: format!("dev:{device}"),
            available: available_bytes(existing)?,
        })
    }
}

pub(crate) fn resolve_existing(path: &Path) -> Result<PathBuf, String> {
    if path.exists() {
        return path.canonicalize().map_err(|e| e.to_string());
    }
    let parent = path.parent().ok_or("目录不可用")?;
    Ok(resolve_existing(parent)?.join(path.file_name().ok_or("目录无效")?))
}

#[cfg(target_os = "windows")]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetDiskFreeSpaceExW(
        lpDirectoryName: *const u16,
        lpFreeBytesAvailableToCaller: *mut u64,
        lpTotalNumberOfBytes: *mut u64,
        lpTotalNumberOfFreeBytes: *mut u64,
    ) -> i32;
}

pub fn available_bytes(path: &Path) -> Result<u64, String> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::ffi::OsStrExt;
        let mut current_path = path;
        while !current_path.exists() {
            if let Some(parent) = current_path.parent() {
                current_path = parent;
            } else {
                break;
            }
        }
        let wide_path: Vec<u16> = current_path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        let mut free_bytes: u64 = 0;
        let mut total_bytes: u64 = 0;
        let mut total_free_bytes: u64 = 0;

        let res = unsafe {
            GetDiskFreeSpaceExW(
                wide_path.as_ptr(),
                &mut free_bytes,
                &mut total_bytes,
                &mut total_free_bytes,
            )
        };

        if res == 0 {
            let err = std::io::Error::last_os_error();
            return Err(format!("获取磁盘剩余空间失败: {}", err));
        }

        Ok(free_bytes)
    }

    #[cfg(not(target_os = "windows"))]
    {
        use std::ffi::CString;
        use std::mem::MaybeUninit;

        let mut current_path = path;
        while !current_path.exists() {
            if let Some(parent) = current_path.parent() {
                current_path = parent;
            } else {
                break;
            }
        }
        let check_path = if current_path.exists() {
            current_path
        } else {
            std::path::Path::new(".")
        };

        let c_path = CString::new(check_path.to_string_lossy().as_bytes())
            .map_err(|e| format!("路径包含非法字符: {}", e))?;

        unsafe {
            let mut stat: MaybeUninit<libc::statvfs> = MaybeUninit::uninit();
            if libc::statvfs(c_path.as_ptr(), stat.as_mut_ptr()) == 0 {
                let stat = stat.assume_init();
                // f_bavail 是普通用户可用块数，f_frsize 为基本文件系统块大小
                let free_bytes = (stat.f_bavail as u64).saturating_mul(stat.f_frsize as u64);
                Ok(free_bytes)
            } else {
                let err = std::io::Error::last_os_error();
                Err(format!("获取磁盘剩余空间失败: {}", err))
            }
        }
    }
}
