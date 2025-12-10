use anyhow::Result;
use std::path::{Path, PathBuf};
use wasmtime_wasi::{DirPerms, FilePerms};

/// Configuration for the WasmRuntime
/// Includes settings for the WASI environment
/// such as directory and file permissions, root directory, etc.
/// Also includes settings for the runtime itself
/// such as the host path, guest path, etc.
/// These settings are used when creating the WASI context
/// for each module execution
/// The default settings are:
/// - host path: current working directory on the host system (e.g. "/home/user")
/// - guest path: "."
/// - root directory: current working directory on the host system (e.g. "/home/user")
/// - directory permissions: all
/// - file permissions: all
/// - allow write access: false
/// - wasm file extension: "wasm"
#[derive(Clone, Debug)]
pub struct WasmConfig {
    host_path: PathBuf,
    guest_path: String,
    dir_perms: DirPerms,
    file_perms: FilePerms,
    rootdir: PathBuf,
    wasm_ext: String,

    allow_write: bool,
    allow_network: bool,
}

impl Default for WasmConfig {
    fn default() -> Self {
        Self {
            host_path: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            guest_path: "/".to_string(),
            dir_perms: DirPerms::all(),
            file_perms: FilePerms::all(),
            rootdir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            allow_write: false,
            wasm_ext: "wasm".to_string(),
            allow_network: false,
        }
    }
}

/// Methods for WasmConfig
impl WasmConfig {
    /// Allow network access
    /// Default: false
    pub fn set_allow_network(&mut self, allow: bool) -> &Self {
        self.allow_network = allow;
        self
    }

    pub fn get_allow_network(&self) -> bool {
        self.allow_network
    }

    /// Create a new WasmConfig with default settings
    /// Default host path: current working directory on the host system (e.g. "/home/user")
    /// Default guest path: "."
    /// Default root directory: current working directory on the host system (e.g. "/home
    pub fn set_rootdir<P: AsRef<Path>>(&mut self, p: P) -> &Self {
        self.rootdir = p.as_ref().to_path_buf();
        self
    }

    /// Set the host path
    /// Default: current working directory on the host system (e.g. "/home/user")
    /// This is the path on the host system that maps to the guest path
    /// e.g. if host_path is "/tmp" and guest_path is "/data", then
    /// the WASI module will see "/data" as "/tmp" on the host system
    /// Note: host_path must be an absolute path
    pub fn set_host_path<P: AsRef<Path>>(&mut self, p: P) -> Result<&Self> {
        self.host_path = if p.as_ref().is_absolute() { p.as_ref().to_path_buf() } else { self.rootdir.join(p) };
        Ok(self)
    }

    /// Set the guest path
    /// Default: "."
    /// This is the path inside the WASI module that maps to the host path
    /// e.g. if host_path is "/tmp" and guest_path is "/data", then
    /// the WASI module will see "/data" as "/tmp" on the host system
    /// Note: guest_path must be an absolute path or "."
    pub fn set_guest_path<S: AsRef<str>>(&mut self, s: S) -> &Self {
        self.guest_path = s.as_ref().to_string();
        self
    }

    /// Set the wasm file extension (without dot)
    /// Default: "wasm"
    /// Example: "wasm", "wat"
    /// Note: this does not affect loading, only the `objects` method
    /// which lists available wasm modules in the root directory
    /// The runtime will look for files with this extension in the root directory
    /// to list available wasm modules.
    pub fn set_wasm_ext<S: AsRef<str>>(&mut self, s: S) -> &Self {
        self.wasm_ext = s.as_ref().to_string();
        self
    }

    /// Get the wasm file extension (without dot)
    /// Default: "wasm"
    /// Example: "wasm", "wat"
    /// Note: this does not affect loading, only the `objects` method
    /// which lists available wasm modules in the root directory
    /// The runtime will look for files with this extension in the root directory
    /// to list available wasm modules.
    pub fn get_wasm_ext(&self) -> &str {
        &self.wasm_ext
    }

    /// Allow write access to the guest path
    /// Default: false
    pub fn set_allow_write(&mut self, allow: bool) -> &Self {
        self.allow_write = allow;
        self
    }

    /// Set directory permissions
    /// Default: all
    /// Caveat: this does not affect write access, see `set_allow_write`
    pub fn set_dir_perms(&mut self, perms: DirPerms) -> &Self {
        self.dir_perms = perms;
        self
    }

    /// Set file permissions
    /// Default: all
    /// Caveat: this does not affect write access, see `set_allow_write`
    pub fn set_file_perms(&mut self, perms: FilePerms) -> &Self {
        self.file_perms = perms;
        self
    }

    /// Get whether write access to the guest path is allowed
    /// Default: false
    pub fn get_allow_write(&self) -> bool {
        self.allow_write
    }

    /// Get the host path
    /// Default: current working directory on the host system (e.g. "/home/user")
    /// This is the path on the host system that maps to the guest path
    /// e.g. if host_path is "/tmp" and guest_path is "/data", then
    /// the WASI module will see "/data" as "/tmp" on the host system
    /// Note: host_path must be an absolute path
    pub fn get_host_path(&self) -> &Path {
        &self.host_path
    }

    /// Get the guest path
    /// Default: "."
    /// This is the path inside the WASI module that maps to the host path
    /// e.g. if host_path is "/tmp" and guest_path is "/data", then
    /// the WASI module will see "/data" as "/tmp" on the host system
    /// Note: guest_path must be an absolute path or "."
    pub fn get_guest_path(&self) -> &str {
        &self.guest_path
    }

    /// Get the root directory where wasm modules are stored
    /// Default: current working directory on the host system (e.g. "/home/user")
    /// This is the directory where the runtime will look for wasm modules to load
    /// e.g. if rootdir is "/home/user/wasm", then the runtime will look for wasm modules in "/home/user/wasm"
    /// Note: rootdir must be an absolute path
    pub fn get_root_path(&self) -> &Path {
        &self.rootdir
    }

    /// Get directory permissions
    /// Default: all
    /// Caveat: this does not affect write access, see `set_allow_write`
    pub fn get_dir_perms(&self) -> DirPerms {
        self.dir_perms
    }

    /// Get file permissions
    /// Default: all
    /// Caveat: this does not affect write access, see `set_allow_write`
    pub fn get_file_perms(&self) -> FilePerms {
        self.file_perms
    }
}
