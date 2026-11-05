pub mod client;
pub mod protocol;
mod security;
pub(crate) mod server;

pub use protocol::{IpcRequest, decode_frame, encode_frame};
pub use security::CurrentUserAcl;
pub use server::IpcServer;

use crate::Result;
use crate::platform::{last_error, wide_null};
use windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows_sys::Win32::System::Threading::GetCurrentProcessId;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstanceNames {
    pub mutex: Vec<u16>,
    pub pipe: Vec<u16>,
}

impl InstanceNames {
    /// This session's names. A portable FastPad's are its own for each data folder, so it never
    /// hands a file to an installed FastPad (or one from another folder) with other settings.
    pub fn for_current_session() -> Result<Self> {
        let session = session_of(unsafe { GetCurrentProcessId() })?;
        Ok(match crate::platform::portable_dir() {
            Some(dir) => Self::for_portable_session(session, dir),
            None => Self::for_session(session),
        })
    }

    pub fn for_session(session: u32) -> Self {
        Self::named(&format!("FastPad-{session}"))
    }

    /// Names for a portable FastPad keeping its data in `data_dir`.
    pub fn for_portable_session(session: u32, data_dir: &std::path::Path) -> Self {
        Self::named(&format!(
            "FastPad-{session}-portable-{:016x}",
            folder_hash(data_dir)
        ))
    }

    fn named(name: &str) -> Self {
        Self {
            mutex: wide_null(&format!(r"Local\{name}")),
            pipe: wide_null(&format!(r"\\.\pipe\{name}")),
        }
    }
}

/// FNV-1a over the folder's path, lowercased: Windows paths are case-insensitive, and the same
/// folder must give the same names on every launch.
fn folder_hash(path: &std::path::Path) -> u64 {
    path.to_string_lossy()
        .to_lowercase()
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
        })
}

fn session_of(process_id: u32) -> Result<u32> {
    let mut session = 0;
    if unsafe { ProcessIdToSessionId(process_id, &mut session) } == 0 {
        return Err(last_error());
    }
    Ok(session)
}

/// Binds the listening pipe for this session; used only by deferred window work.
pub fn bind_session_server() -> Result<IpcServer> {
    IpcServer::bind(
        &InstanceNames::for_current_session()?,
        &CurrentUserAcl::current()?,
    )
}

#[cfg(test)]
mod tests {
    use super::InstanceNames;
    use crate::platform::wide_null;

    #[test]
    fn names_are_scoped_to_the_session_id() {
        // Break caught: a global or per-user name lets two interactive sessions steal launches.
        let names = InstanceNames::for_session(3);
        assert_eq!(names.mutex, wide_null(r"Local\FastPad-3"));
        assert_eq!(names.pipe, wide_null(r"\\.\pipe\FastPad-3"));
    }

    #[test]
    fn portable_names_are_their_own_for_each_data_folder() {
        // Break caught: a portable FastPad forwarding its files to the installed one (whose
        // settings and session are elsewhere), or two launches from one folder not finding
        // each other because the folder was spelled in another case.
        use std::path::Path;
        let installed = InstanceNames::for_session(3);
        let a = InstanceNames::for_portable_session(3, Path::new(r"D:\Tools\FastPad\data"));
        let a_again = InstanceNames::for_portable_session(3, Path::new(r"d:\tools\fastpad\DATA"));
        let b = InstanceNames::for_portable_session(3, Path::new(r"E:\FastPad\data"));
        assert_ne!(a, installed);
        assert_ne!(a, b);
        assert_eq!(a, a_again);
    }

    #[test]
    fn current_session_names_use_this_process_session() {
        let names = InstanceNames::for_current_session().unwrap();
        let mutex = String::from_utf16(&names.mutex[..names.mutex.len() - 1]).unwrap();
        let session = mutex.strip_prefix(r"Local\FastPad-").unwrap();
        assert_eq!(names, InstanceNames::for_session(session.parse().unwrap()));
    }
}
