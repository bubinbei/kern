//! Windows-only OS boundary. File owns the handle; all I/O is synchronous and
//! exclusive. Stack DCB/timeouts live through their calls; no raw pointer is stored.
use kern_im2480::{Port, normalize_port};
use std::{
    fs::{File, OpenOptions},
    io::{self, Read, Write},
    os::windows::{fs::OpenOptionsExt, io::AsRawHandle},
};
use windows_sys::Win32::{Devices::Communication::*, Foundation::HANDLE};
pub struct NativePort {
    file: File,
}
impl NativePort {
    pub fn available_ports() -> Vec<String> {
        let mut ports = [0u32; 256];
        let mut count = 0;
        // SAFETY: fixed output buffer and its matching length; enumeration opens no ports.
        let result = unsafe { GetCommPorts(ports.as_mut_ptr(), ports.len() as u32, &mut count) };
        if result != 0 {
            return Vec::new();
        }
        ports[..(count as usize).min(ports.len())]
            .iter()
            .map(|n| format!("COM{n}"))
            .collect()
    }
    pub fn open(port: &str, baud: u32) -> io::Result<Self> {
        let port = normalize_port(port)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0)
            .open(format!(r"\\.\{port}"))?;
        let handle = file.as_raw_handle() as HANDLE;
        // SAFETY: valid, exclusively owned COM handle and initialized stack buffers.
        unsafe {
            let mut dcb: DCB = std::mem::zeroed();
            dcb.DCBlength = std::mem::size_of::<DCB>() as u32;
            if GetCommState(handle, &mut dcb) == 0 {
                return Err(io::Error::last_os_error());
            }
            let mode: Vec<u16> = format!("baud={baud} parity=n data=8 stop=1 dtr=off rts=off")
                .encode_utf16()
                .chain(Some(0))
                .collect();
            if BuildCommDCBW(mode.as_ptr(), &mut dcb) == 0 || SetCommState(handle, &dcb) == 0 {
                return Err(io::Error::last_os_error());
            }
            let timeouts = COMMTIMEOUTS {
                ReadIntervalTimeout: u32::MAX,
                ReadTotalTimeoutMultiplier: u32::MAX,
                ReadTotalTimeoutConstant: 25,
                WriteTotalTimeoutMultiplier: 0,
                WriteTotalTimeoutConstant: 100,
            };
            if SetCommTimeouts(handle, &timeouts) == 0 {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(Self { file })
    }
}
impl Read for NativePort {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.file.read(bytes)
    }
}
impl Write for NativePort {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.file.write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl Port for NativePort {
    fn clear_input(&mut self) -> io::Result<()> {
        // SAFETY: File retains ownership of this synchronous COM handle.
        if unsafe { PurgeComm(self.file.as_raw_handle() as HANDLE, PURGE_RXCLEAR) } == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}
