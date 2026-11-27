// Linux UAPI, linux/fscrypt.h. Only v2 policies and filesystem-level keys.
use hkdf::Hkdf;
use nix::libc;
use sha2::Sha512;
use std::{fs::File, io, os::fd::AsRawFd, path::Path};
use zeroize::Zeroizing;

const ADD: libc::c_ulong = nix::request_code_readwrite!(b'f', 23, 80) as _;
const REMOVE: libc::c_ulong = nix::request_code_readwrite!(b'f', 24, 64) as _;
const SET_POLICY: libc::c_ulong = nix::request_code_read!(b'f', 19, 12) as _;

pub fn identifier(key: &[u8; 64]) -> [u8; 16] {
    let mut id = [0; 16];
    Hkdf::<Sha512>::new(None, key)
        .expand(b"fscrypt\0\x01", &mut id)
        .unwrap();
    id
}

fn ioctl(dir: &File, op: libc::c_ulong, bytes: &mut [u8]) -> io::Result<()> {
    // Each caller supplies the full fixed UAPI structure including any trailing key.
    if unsafe { libc::ioctl(dir.as_raw_fd(), op, bytes.as_mut_ptr()) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub fn unlock(root: &File, key: &[u8; 64]) -> io::Result<()> {
    let mut arg = Zeroizing::new([0u8; 144]);
    arg[..4].copy_from_slice(&2u32.to_ne_bytes());
    arg[40..44].copy_from_slice(&64u32.to_ne_bytes());
    arg[80..].copy_from_slice(key);
    ioctl(root, ADD, &mut arg[..])?;
    if arg[8..24] != identifier(key) {
        let actual = arg[8..24].try_into().unwrap();
        lock(root, &actual)?;
        return Err(io::Error::other("Unexpected fscrypt key identifier"));
    }
    Ok(())
}

pub fn apply(path: &Path, id: &[u8; 16]) -> io::Result<()> {
    let mut policy = [0; 24];
    policy[..4].copy_from_slice(&[2, 1, 4, 3]);
    policy[8..].copy_from_slice(id);
    ioctl(&File::open(path)?, SET_POLICY, &mut policy)
}

pub fn lock(root: &File, id: &[u8; 16]) -> io::Result<()> {
    let mut arg = [0; 64];
    arg[..4].copy_from_slice(&2u32.to_ne_bytes());
    arg[8..24].copy_from_slice(id);
    for _ in 0..10 {
        match ioctl(root, REMOVE, &mut arg) {
            Err(e) if e.raw_os_error() == Some(libc::ENOKEY) => (),
            Err(e) => return Err(e),
            Ok(()) => (),
        }
        let mut status = [0; 128];
        status[..40].copy_from_slice(&arg[..40]);
        ioctl(
            root,
            nix::request_code_readwrite!(b'f', 26, 128) as _,
            &mut status,
        )?;
        if u32::from_ne_bytes(status[64..68].try_into().unwrap()) == 1 {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
        arg[40..].fill(0);
    }
    Err(io::Error::other("fscrypt key eviction incomplete"))
}
