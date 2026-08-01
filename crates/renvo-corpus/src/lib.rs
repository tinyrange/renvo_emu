//! Docker-isolated compiler corpus builds and artifact provenance.

mod compare;
mod matrix;
mod reduce;

pub use compare::{Comparison, Difference, NamedObservation, compare_observations};
pub use matrix::{BuildVariant, CompilerMatrix};
pub use reduce::{
    CaseReductionResult, ReductionCandidate, ReductionResult, reduce_case, reduce_sequence,
    try_reduce_sequence,
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use thiserror::Error;

/// Versioned compiler toolchain configuration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolchainSpec {
    /// Stable toolchain name used in case manifests.
    pub name: String,
    /// Immutable Docker image reference or image ID.
    pub image: String,
    /// Optional locally built tag used when the recorded image ID is absent.
    /// It is resolved to an immutable ID before the container is started.
    #[serde(default)]
    pub local_image: Option<String>,
    /// Compiler or build program inside the container.
    pub program: String,
    /// Arguments placed before request-specific arguments.
    #[serde(default)]
    pub args: Vec<String>,
    /// Explicit build environment.
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
}

impl ToolchainSpec {
    /// Validates the security and reproducibility contract.
    pub fn validate(&self) -> Result<(), CorpusError> {
        if self.name.trim().is_empty() {
            return Err(CorpusError::InvalidSpec(
                "toolchain name must not be empty".to_owned(),
            ));
        }
        if self.program.trim().is_empty() {
            return Err(CorpusError::InvalidSpec(
                "toolchain program must not be empty".to_owned(),
            ));
        }
        if !is_immutable_image_reference(&self.image) {
            return Err(CorpusError::MutableImageReference(self.image.clone()));
        }
        if let Some(local_image) = &self.local_image
            && !is_safe_local_image_reference(local_image)
        {
            return Err(CorpusError::InvalidSpec(format!(
                "invalid local Docker image fallback {local_image:?}"
            )));
        }
        for key in self.environment.keys() {
            if key.is_empty()
                || !key
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '_')
            {
                return Err(CorpusError::InvalidSpec(format!(
                    "invalid environment name {key:?}"
                )));
            }
        }
        Ok(())
    }
}

/// Resource limits applied to one compiler container.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DockerLimits {
    /// Build wall-time limit.
    pub timeout_seconds: u64,
    /// Docker memory limit syntax, for example `1g`.
    pub memory: String,
    /// Maximum process count.
    pub pids: u32,
    /// CPU quota accepted by `docker run --cpus`.
    pub cpus: String,
    /// Optional numeric `uid:gid`.
    pub user: Option<String>,
}

impl Default for DockerLimits {
    fn default() -> Self {
        Self {
            timeout_seconds: 120,
            memory: "1g".to_owned(),
            pids: 256,
            cpus: "2".to_owned(),
            user: None,
        }
    }
}

/// One isolated build invocation.
#[derive(Clone, Debug)]
pub struct BuildRequest {
    /// Toolchain container and program.
    pub toolchain: ToolchainSpec,
    /// Host source tree mounted read-only at `/workspace/src`.
    pub source_dir: PathBuf,
    /// Host output directory mounted read-write at `/workspace/out`.
    pub output_dir: PathBuf,
    /// Arguments placed after toolchain arguments.
    pub arguments: Vec<String>,
    /// Architecture/compiler target recorded in provenance.
    pub target: String,
    /// Resource and wall-time limits.
    pub limits: DockerLimits,
}

/// Content hash for one relative input or output path.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileHash {
    /// Slash-normalized relative path.
    pub path: String,
    /// Lowercase SHA-256.
    pub sha256: String,
    /// File size in bytes.
    pub size: u64,
}

/// Complete provenance and output of one build.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildArtifact {
    /// Schema identifier for forwards-compatible readers.
    pub schema: String,
    /// Stable toolchain name.
    pub toolchain: String,
    /// Image reference selected from the recorded ID or local fallback.
    pub image: String,
    /// Docker's locally resolved immutable image ID.
    pub image_id: String,
    /// Compiler target recorded by the case.
    pub target: String,
    /// Complete in-container argv.
    pub argv: Vec<String>,
    /// Explicit environment in sorted order.
    pub environment: BTreeMap<String, String>,
    /// Sorted source content hashes.
    pub inputs: Vec<FileHash>,
    /// Sorted output content hashes.
    pub outputs: Vec<FileHash>,
    /// Container exit code, or `-1` when terminated by signal.
    pub exit_code: i32,
    /// Captured standard output.
    pub stdout: String,
    /// Captured standard error.
    pub stderr: String,
    /// True when Renvo killed the build at its wall-time limit.
    pub timed_out: bool,
}

impl BuildArtifact {
    /// True when the compiler container exited successfully.
    pub const fn succeeded(&self) -> bool {
        self.exit_code == 0 && !self.timed_out
    }

    /// Writes pretty, stable JSON.
    pub fn write_json(&self, path: &Path) -> Result<(), CorpusError> {
        let bytes = serde_json::to_vec_pretty(self)?;
        fs::write(path, bytes).map_err(|source| CorpusError::Io {
            path: path.to_path_buf(),
            source,
        })
    }
}

/// Docker-backed corpus compiler.
#[derive(Clone, Debug)]
pub struct DockerCompiler {
    executable: PathBuf,
}

impl Default for DockerCompiler {
    fn default() -> Self {
        Self::new("docker")
    }
}

impl DockerCompiler {
    /// Uses a Docker-compatible executable.
    pub fn new(executable: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
        }
    }

    /// Confirms that a Docker server is reachable.
    pub fn verify_available(&self) -> Result<(), CorpusError> {
        let output = Command::new(&self.executable)
            .args(["info", "--format", "{{.ServerVersion}}"])
            .output()
            .map_err(|source| CorpusError::DockerLaunch {
                executable: self.executable.clone(),
                source,
            })?;
        if !output.status.success() {
            return Err(CorpusError::DockerFailure {
                operation: "docker info".to_owned(),
                status: status_code(output.status),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            });
        }
        Ok(())
    }

    /// Returns Docker arguments without launching a container.
    pub fn command(&self, request: &BuildRequest) -> Result<Vec<String>, CorpusError> {
        self.command_with_image(request, &request.toolchain.image)
    }

    fn command_with_image(
        &self,
        request: &BuildRequest,
        image: &str,
    ) -> Result<Vec<String>, CorpusError> {
        request.toolchain.validate()?;
        validate_limits(&request.limits)?;
        let source = canonical_directory(&request.source_dir)?;
        fs::create_dir_all(&request.output_dir).map_err(|source_error| CorpusError::Io {
            path: request.output_dir.clone(),
            source: source_error,
        })?;
        let output = canonical_directory(&request.output_dir)?;
        if source == output {
            return Err(CorpusError::InvalidSpec(
                "source and output directories must differ".to_owned(),
            ));
        }

        let mut arguments = vec![
            "run".to_owned(),
            "--rm".to_owned(),
            "--pull=never".to_owned(),
            "--network=none".to_owned(),
            "--read-only".to_owned(),
            "--cap-drop=ALL".to_owned(),
            "--security-opt=no-new-privileges".to_owned(),
            format!("--memory={}", request.limits.memory),
            format!("--pids-limit={}", request.limits.pids),
            format!("--cpus={}", request.limits.cpus),
            "--tmpfs=/tmp:rw,noexec,nosuid,size=64m".to_owned(),
            format!(
                "--mount=type=bind,src={},dst=/workspace/src,readonly",
                source.display()
            ),
            format!(
                "--mount=type=bind,src={},dst=/workspace/out",
                output.display()
            ),
            "--workdir=/workspace/src".to_owned(),
        ];
        if let Some(user) = container_user(&output, request.limits.user.as_deref())? {
            arguments.push(format!("--user={user}"));
        }
        for (key, value) in &request.toolchain.environment {
            arguments.push("--env".to_owned());
            arguments.push(format!("{key}={value}"));
        }
        arguments.push(image.to_owned());
        arguments.push(request.toolchain.program.clone());
        arguments.extend(request.toolchain.args.iter().cloned());
        arguments.extend(request.arguments.iter().cloned());
        Ok(arguments)
    }

    /// Runs an isolated compiler container and captures provenance.
    pub fn compile(&self, request: &BuildRequest) -> Result<BuildArtifact, CorpusError> {
        self.verify_available()?;
        request.toolchain.validate()?;
        let (image, image_id) = self.resolve_image(&request.toolchain)?;
        let docker_arguments = self.command_with_image(request, &image_id)?;
        let inputs = hash_tree(&request.source_dir)?;

        let mut child = Command::new(&self.executable)
            .args(&docker_arguments)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|source| CorpusError::DockerLaunch {
                executable: self.executable.clone(),
                source,
            })?;
        let mut stdout = child.stdout.take().ok_or_else(|| {
            CorpusError::InvalidSpec("compiler stdout pipe was not created".to_owned())
        })?;
        let mut stderr = child.stderr.take().ok_or_else(|| {
            CorpusError::InvalidSpec("compiler stderr pipe was not created".to_owned())
        })?;
        let stdout_reader = thread::spawn(move || {
            let mut bytes = Vec::new();
            stdout.read_to_end(&mut bytes).map(|_| bytes)
        });
        let stderr_reader = thread::spawn(move || {
            let mut bytes = Vec::new();
            stderr.read_to_end(&mut bytes).map(|_| bytes)
        });
        let deadline = Instant::now() + Duration::from_secs(request.limits.timeout_seconds.max(1));
        let timed_out = loop {
            if child
                .try_wait()
                .map_err(|source| CorpusError::DockerLaunch {
                    executable: self.executable.clone(),
                    source,
                })?
                .is_some()
            {
                break false;
            }
            if Instant::now() >= deadline {
                child.kill().map_err(|source| CorpusError::DockerLaunch {
                    executable: self.executable.clone(),
                    source,
                })?;
                break true;
            }
            thread::sleep(Duration::from_millis(10));
        };
        let status = child.wait().map_err(|source| CorpusError::DockerLaunch {
            executable: self.executable.clone(),
            source,
        })?;
        let stdout = stdout_reader
            .join()
            .map_err(|_| CorpusError::InvalidSpec("compiler stdout reader panicked".to_owned()))?
            .map_err(|source| CorpusError::DockerLaunch {
                executable: self.executable.clone(),
                source,
            })?;
        let stderr = stderr_reader
            .join()
            .map_err(|_| CorpusError::InvalidSpec("compiler stderr reader panicked".to_owned()))?
            .map_err(|source| CorpusError::DockerLaunch {
                executable: self.executable.clone(),
                source,
            })?;
        let argv = request
            .toolchain
            .args
            .iter()
            .chain(request.arguments.iter())
            .cloned()
            .collect::<Vec<_>>();
        let mut full_argv = vec![request.toolchain.program.clone()];
        full_argv.extend(argv);
        Ok(BuildArtifact {
            schema: "renvo.build-artifact.v1".to_owned(),
            toolchain: request.toolchain.name.clone(),
            image,
            image_id,
            target: request.target.clone(),
            argv: full_argv,
            environment: request.toolchain.environment.clone(),
            inputs,
            outputs: hash_tree(&request.output_dir)?,
            exit_code: status_code(status),
            stdout: String::from_utf8_lossy(&stdout).into_owned(),
            stderr: String::from_utf8_lossy(&stderr).into_owned(),
            timed_out,
        })
    }

    fn inspect_image(&self, image: &str) -> Result<String, CorpusError> {
        let output = Command::new(&self.executable)
            .args(["image", "inspect", "--format", "{{.Id}}", image])
            .output()
            .map_err(|source| CorpusError::DockerLaunch {
                executable: self.executable.clone(),
                source,
            })?;
        if !output.status.success() {
            return Err(CorpusError::DockerFailure {
                operation: format!("inspect image {image}"),
                status: status_code(output.status),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            });
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    }

    fn resolve_image(&self, spec: &ToolchainSpec) -> Result<(String, String), CorpusError> {
        match self.inspect_image(&spec.image) {
            Ok(image_id) => Ok((spec.image.clone(), image_id)),
            Err(primary_error) => {
                let Some(local_image) = &spec.local_image else {
                    return Err(primary_error);
                };
                self.inspect_image(local_image)
                    .map(|image_id| (local_image.clone(), image_id))
            }
        }
    }
}

/// Corpus or container failure.
#[derive(Debug, Error)]
pub enum CorpusError {
    /// Toolchain or request violates the explicit contract.
    #[error("invalid corpus specification: {0}")]
    InvalidSpec(String),
    /// Image tags are not reproducible.
    #[error("Docker image reference must be immutable (digest or image ID): {0}")]
    MutableImageReference(String),
    /// File operation failed.
    #[error("I/O failed at {path}: {source}")]
    Io {
        /// Affected path.
        path: PathBuf,
        /// Underlying failure.
        #[source]
        source: std::io::Error,
    },
    /// Symlinks are rejected from hashed input/output trees.
    #[error("symlink is not permitted in a corpus tree: {0}")]
    Symlink(PathBuf),
    /// Docker executable could not be launched or controlled.
    #[error("failed to launch {executable}: {source}")]
    DockerLaunch {
        /// Configured executable.
        executable: PathBuf,
        /// Underlying process error.
        #[source]
        source: std::io::Error,
    },
    /// Docker returned a non-success result for a control operation.
    #[error("{operation} failed with status {status}: {stderr}")]
    DockerFailure {
        /// Operation description.
        operation: String,
        /// Process exit code.
        status: i32,
        /// Captured standard error.
        stderr: String,
    },
    /// JSON artifact serialization failed.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

fn is_immutable_image_reference(image: &str) -> bool {
    let digest = image
        .strip_prefix("sha256:")
        .or_else(|| image.split_once("@sha256:").map(|(_, digest)| digest));
    digest.is_some_and(|digest| {
        digest.len() == 64
            && digest
                .chars()
                .all(|character| character.is_ascii_hexdigit())
    })
}

fn is_safe_local_image_reference(image: &str) -> bool {
    !image.is_empty()
        && !image.starts_with('-')
        && !image.ends_with(":latest")
        && !image.contains('@')
        && image.contains(':')
        && image.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '/' | '_' | '-' | ':')
        })
}

fn validate_limits(limits: &DockerLimits) -> Result<(), CorpusError> {
    if limits.timeout_seconds == 0 || limits.pids == 0 {
        return Err(CorpusError::InvalidSpec(
            "Docker timeout and PID limit must be non-zero".to_owned(),
        ));
    }
    if limits.memory.is_empty() || limits.cpus.is_empty() {
        return Err(CorpusError::InvalidSpec(
            "Docker memory and CPU limits must be explicit".to_owned(),
        ));
    }
    Ok(())
}

fn canonical_directory(path: &Path) -> Result<PathBuf, CorpusError> {
    let canonical = path.canonicalize().map_err(|source| CorpusError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let metadata = fs::metadata(&canonical).map_err(|source| CorpusError::Io {
        path: canonical.clone(),
        source,
    })?;
    if !metadata.is_dir() {
        return Err(CorpusError::InvalidSpec(format!(
            "{} is not a directory",
            path.display()
        )));
    }
    Ok(canonical)
}

#[cfg(unix)]
fn container_user(output: &Path, explicit: Option<&str>) -> Result<Option<String>, CorpusError> {
    use std::os::unix::fs::MetadataExt;

    if let Some(explicit) = explicit {
        return Ok(Some(explicit.to_owned()));
    }
    let metadata = fs::metadata(output).map_err(|source| CorpusError::Io {
        path: output.to_path_buf(),
        source,
    })?;
    Ok(Some(format!("{}:{}", metadata.uid(), metadata.gid())))
}

#[cfg(not(unix))]
fn container_user(_output: &Path, explicit: Option<&str>) -> Result<Option<String>, CorpusError> {
    Ok(explicit.map(str::to_owned))
}

fn hash_tree(root: &Path) -> Result<Vec<FileHash>, CorpusError> {
    let root = canonical_directory(root)?;
    let mut paths = Vec::new();
    collect_files(&root, &root, &mut paths)?;
    paths.sort();
    paths
        .into_iter()
        .map(|relative| {
            let path = root.join(&relative);
            let bytes = fs::read(&path).map_err(|source| CorpusError::Io {
                path: path.clone(),
                source,
            })?;
            Ok(FileHash {
                path: relative
                    .components()
                    .map(|component| component.as_os_str().to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("/"),
                sha256: hex::encode(Sha256::digest(&bytes)),
                size: bytes.len() as u64,
            })
        })
        .collect()
}

fn collect_files(root: &Path, current: &Path, paths: &mut Vec<PathBuf>) -> Result<(), CorpusError> {
    let mut entries = fs::read_dir(current)
        .map_err(|source| CorpusError::Io {
            path: current.to_path_buf(),
            source,
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| CorpusError::Io {
            path: current.to_path_buf(),
            source,
        })?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        let file_type = entry.file_type().map_err(|source| CorpusError::Io {
            path: path.clone(),
            source,
        })?;
        if file_type.is_symlink() {
            return Err(CorpusError::Symlink(path));
        }
        if file_type.is_dir() {
            collect_files(root, &path, paths)?;
        } else if file_type.is_file() {
            paths.push(
                path.strip_prefix(root)
                    .expect("recursive path begins with root")
                    .to_path_buf(),
            );
        }
    }
    Ok(())
}

fn status_code(status: ExitStatus) -> i32 {
    status.code().unwrap_or(-1)
}

#[cfg(test)]
mod tests {
    use super::*;

    const IMAGE: &str = "example.invalid/riscv-gcc@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn rejects_mutable_image_tags() {
        let spec = ToolchainSpec {
            name: "gcc".to_owned(),
            image: "gcc:latest".to_owned(),
            local_image: None,
            program: "gcc".to_owned(),
            args: vec![],
            environment: BTreeMap::new(),
        };
        assert!(matches!(
            spec.validate(),
            Err(CorpusError::MutableImageReference(_))
        ));
    }

    #[test]
    fn command_has_isolation_controls_and_stable_environment_order() {
        let source = tempfile::tempdir().unwrap();
        let output = tempfile::tempdir().unwrap();
        let mut environment = BTreeMap::new();
        environment.insert("Z_FLAG".to_owned(), "last".to_owned());
        environment.insert("A_FLAG".to_owned(), "first".to_owned());
        let request = BuildRequest {
            toolchain: ToolchainSpec {
                name: "gcc".to_owned(),
                image: IMAGE.to_owned(),
                local_image: None,
                program: "riscv-none-elf-gcc".to_owned(),
                args: vec!["-ffreestanding".to_owned()],
                environment,
            },
            source_dir: source.path().to_path_buf(),
            output_dir: output.path().to_path_buf(),
            arguments: vec![
                "main.c".to_owned(),
                "-o".to_owned(),
                "/workspace/out/a.elf".to_owned(),
            ],
            target: "rv32ec".to_owned(),
            limits: DockerLimits::default(),
        };
        let command = DockerCompiler::default().command(&request).unwrap();
        assert!(command.contains(&"--network=none".to_owned()));
        assert!(command.contains(&"--read-only".to_owned()));
        assert!(command.contains(&"--cap-drop=ALL".to_owned()));
        assert!(command.contains(&"--pull=never".to_owned()));
        let a = command
            .iter()
            .position(|arg| arg == "A_FLAG=first")
            .unwrap();
        let z = command.iter().position(|arg| arg == "Z_FLAG=last").unwrap();
        assert!(a < z);
        assert_eq!(
            command.last().unwrap(),
            "/workspace/out/a.elf",
            "request arguments follow the program"
        );
    }

    #[test]
    fn accepts_a_named_local_fallback_but_not_latest() {
        let mut spec = ToolchainSpec {
            name: "gcc".to_owned(),
            image: IMAGE.to_owned(),
            local_image: Some("renvo/cross-gcc:local".to_owned()),
            program: "gcc".to_owned(),
            args: vec![],
            environment: BTreeMap::new(),
        };
        assert!(spec.validate().is_ok());
        spec.local_image = Some("gcc:latest".to_owned());
        assert!(matches!(spec.validate(), Err(CorpusError::InvalidSpec(_))));
    }

    #[test]
    fn tree_hashing_is_content_and_path_stable() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("b.c"), "b").unwrap();
        fs::write(root.path().join("a.c"), "a").unwrap();
        let hashes = hash_tree(root.path()).unwrap();
        assert_eq!(hashes[0].path, "a.c");
        assert_eq!(hashes[1].path, "b.c");
        assert_eq!(hashes[0].size, 1);
        assert_eq!(hashes[0].sha256.len(), 64);
    }
}
