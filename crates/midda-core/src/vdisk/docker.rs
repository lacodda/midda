//! What Docker keeps inside its disk, as Docker's engine says it: images,
//! containers, volumes and the build cache, each with what of it nothing uses.
//!
//! Docker Desktop keeps everything in one virtual disk, and the files in it
//! are layers no reader can tell apart. The engine can: `GET /system/df` - what
//! `docker system df` prints - says each image's size and how many containers
//! use it, each volume's and whether anything mounts it. midda asks the engine
//! directly, over the pipe or socket the `docker` command itself talks to,
//! and gets bytes rather than the rounded figures the command prints.
//!
//! What can go is reckoned as Docker reckons it, so the figure here is the one
//! `docker system df` shows beside it: an image no container uses, a stopped
//! container's own layer, a volume nothing mounts, build cache not in use.

use std::io::{Read, Write};
use std::time::Duration;

use serde::Deserialize;

/// The engine's interface version asked for: Docker 25 and later speak it,
/// and its answer has the shape read here. An older engine is asked without
/// a version.
const VERSION: &str = "v1.44";

/// How long the engine has to answer. Measuring volumes reads them, and a
/// large one takes a few seconds; an engine that has not answered in this is
/// not going to.
pub const WITHIN: Duration = Duration::from_secs(60);

/// Where the engine listens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    /// A named pipe on Windows: `\\.\pipe\docker_engine`.
    Pipe(String),
    /// A socket file elsewhere: `/var/run/docker.sock`.
    Socket(String),
    /// `host:port`, unencrypted.
    Tcp(String),
}

impl Endpoint {
    /// The endpoint `DOCKER_HOST` names, or the platform's own when it is not
    /// set - the one the `docker` command talks to unless told otherwise.
    ///
    /// # Errors
    ///
    /// When `DOCKER_HOST` names a transport midda does not speak: ssh, or tcp
    /// with TLS asked for.
    pub fn from(host: Option<&str>, tls: bool) -> Result<Self, String> {
        let Some(host) = host.map(str::trim).filter(|host| !host.is_empty()) else {
            return Ok(if cfg!(windows) {
                Self::Pipe(r"\\.\pipe\docker_engine".to_owned())
            } else {
                Self::Socket("/var/run/docker.sock".to_owned())
            });
        };
        if let Some(pipe) = host.strip_prefix("npipe://") {
            return Ok(Self::Pipe(pipe.replace('/', "\\")));
        }
        if let Some(socket) = host.strip_prefix("unix://") {
            return Ok(Self::Socket(socket.to_owned()));
        }
        if let Some(address) = host.strip_prefix("tcp://") {
            if tls {
                return Err(format!("DOCKER_HOST is {host} with TLS, which midda does not speak"));
            }
            return Ok(Self::Tcp(address.trim_end_matches('/').to_owned()));
        }
        Err(format!("DOCKER_HOST is {host}, which midda does not know how to reach"))
    }

    /// As [`Endpoint::from`], from this process's environment.
    ///
    /// # Errors
    ///
    /// As [`Endpoint::from`].
    pub fn of_environment() -> Result<Self, String> {
        let host = std::env::var("DOCKER_HOST").ok();
        let tls = std::env::var_os("DOCKER_TLS_VERIFY").is_some_and(|value| !value.is_empty() && value != "0");
        Self::from(host.as_deref(), tls)
    }
}

/// One kind of thing Docker keeps.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Part {
    /// How many there are.
    pub count: u64,
    /// What they take.
    pub size: u64,
    /// How many nothing uses, and what those take.
    pub idle: u64,
    pub reclaimable: u64,
}

/// What Docker keeps, by kind.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    /// Every image; reclaimable is what no container's image holds.
    pub images: Part,
    /// The images with no name, the leftovers of a rebuild or a pull under a
    /// name another image took: what of them is their own.
    pub untagged: Part,
    /// Each container's own layer; reclaimable is the stopped ones'.
    pub containers: Part,
    /// Volumes; reclaimable is what no container mounts.
    pub volumes: Part,
    /// The build cache; reclaimable is what no build holds.
    pub build_cache: Part,
}

impl Usage {
    /// What all of it takes.
    #[must_use]
    pub const fn size(&self) -> u64 {
        self.images.size + self.containers.size + self.volumes.size + self.build_cache.size
    }
}

/// The parts of the engine's answer that are read.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Answer {
    #[serde(default)]
    layers_size: i64,
    #[serde(default)]
    images: Option<Vec<Image>>,
    #[serde(default)]
    containers: Option<Vec<Container>>,
    #[serde(default)]
    volumes: Option<Vec<Volume>>,
    #[serde(default)]
    build_cache: Option<Vec<Cache>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Image {
    #[serde(default)]
    repo_tags: Option<Vec<String>>,
    #[serde(default)]
    size: i64,
    #[serde(default)]
    shared_size: i64,
    #[serde(default)]
    containers: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Container {
    #[serde(default)]
    size_rw: i64,
    #[serde(default)]
    state: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Volume {
    #[serde(default)]
    usage_data: Option<VolumeUsage>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct VolumeUsage {
    size: i64,
    ref_count: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Cache {
    #[serde(default)]
    size: i64,
    #[serde(default)]
    in_use: bool,
    #[serde(default)]
    shared: bool,
}

/// A size the engine reports, or nothing when it says it does not know (-1).
fn known(bytes: i64) -> Option<u64> {
    u64::try_from(bytes).ok()
}

/// Reads the engine's answer to `GET /system/df`.
///
/// # Errors
///
/// When it is not JSON of that shape.
pub fn parse(body: &[u8]) -> Result<Usage, String> {
    let answer: Answer = serde_json::from_slice(body).map_err(|error| format!("Docker's answer did not read: {error}"))?;
    let mut usage = Usage::default();

    // An image is in use when a container runs from it; what in use holds
    // is its own layers, the ones no other image shares. Docker reckons the
    // rest of all layers reclaimable, and so does this.
    let images = answer.images.unwrap_or_default();
    let used: u64 = images
        .iter()
        .filter(|image| image.containers > 0)
        .filter_map(|image| Some(known(image.size)?.saturating_sub(known(image.shared_size)?)))
        .sum();
    let layers = known(answer.layers_size).unwrap_or(0);
    usage.images = Part {
        count: images.len() as u64,
        size: layers,
        idle: images.iter().filter(|image| image.containers == 0).count() as u64,
        reclaimable: layers.saturating_sub(used),
    };
    for image in &images {
        let untagged = image.repo_tags.as_deref().is_none_or(|tags| tags.iter().all(|tag| tag == "<none>:<none>"));
        if !untagged {
            continue;
        }
        let own = known(image.size).unwrap_or(0).saturating_sub(known(image.shared_size).unwrap_or(0));
        usage.untagged.count += 1;
        usage.untagged.size += own;
        if image.containers == 0 {
            usage.untagged.idle += 1;
            usage.untagged.reclaimable += own;
        }
    }

    for container in answer.containers.unwrap_or_default() {
        let size = known(container.size_rw).unwrap_or(0);
        usage.containers.count += 1;
        usage.containers.size += size;
        if !matches!(container.state.as_str(), "running" | "paused" | "restarting") {
            usage.containers.idle += 1;
            usage.containers.reclaimable += size;
        }
    }

    for volume in answer.volumes.unwrap_or_default() {
        usage.volumes.count += 1;
        let Some(data) = volume.usage_data else { continue };
        let Some(size) = known(data.size) else { continue };
        usage.volumes.size += size;
        if data.ref_count == 0 {
            usage.volumes.idle += 1;
            usage.volumes.reclaimable += size;
        }
    }

    for cache in answer.build_cache.unwrap_or_default() {
        // A record shared with another is counted under that one.
        if cache.shared {
            continue;
        }
        let size = known(cache.size).unwrap_or(0);
        usage.build_cache.count += 1;
        usage.build_cache.size += size;
        if !cache.in_use {
            usage.build_cache.idle += 1;
            usage.build_cache.reclaimable += size;
        }
    }
    Ok(usage)
}

/// Asks the engine at `endpoint` what it keeps, waiting up to `within`.
///
/// # Errors
///
/// When the engine is not there - Docker is not running - refuses, does not
/// answer in time, or answers with something other than what it keeps. The
/// message is said to the reader as it is.
pub fn ask(endpoint: &Endpoint, within: Duration) -> Result<Usage, String> {
    let endpoint = endpoint.clone();
    let (sender, receiver) = std::sync::mpsc::channel();
    // A pipe has no timeout of its own: the question is asked on a thread of
    // its own, and left to finish by itself if the engine never answers.
    std::thread::spawn(move || {
        let _ = sender.send(ask_now(&endpoint));
    });
    receiver
        .recv_timeout(within)
        .map_err(|_| format!("Docker did not answer in {} seconds", within.as_secs()))?
}

fn ask_now(endpoint: &Endpoint) -> Result<Usage, String> {
    let (status, body) = get(endpoint, &format!("/{VERSION}/system/df"))?;
    // An engine older than the version asked for says so; asked without
    // one, it answers in its own, which has the same fields.
    let (status, body) = if status == 400 && String::from_utf8_lossy(&body).contains("too new") {
        get(endpoint, "/system/df")?
    } else {
        (status, body)
    };
    if status != 200 {
        let said = String::from_utf8_lossy(&body);
        return Err(format!("Docker answered {status}: {}", said.trim().chars().take(300).collect::<String>()));
    }
    parse(&body)
}

/// One `GET`, as HTTP/1.0: the engine answers whole and closes, with no
/// chunks to put together.
fn get(endpoint: &Endpoint, path: &str) -> Result<(u16, Vec<u8>), String> {
    let request = format!("GET {path} HTTP/1.0\r\nHost: docker\r\nUser-Agent: midda\r\nAccept: application/json\r\n\r\n");
    let mut stream = connect(endpoint)?;
    stream
        .write_all(request.as_bytes())
        .map_err(|error| format!("the question could not be sent to Docker: {error}"))?;
    let mut answer = Vec::new();
    stream
        .read_to_end(&mut answer)
        .map_err(|error| format!("Docker's answer could not be read: {error}"))?;
    response(&answer)
}

/// The status and the body of an HTTP response, a chunked one put together.
///
/// # Errors
///
/// When it is not an HTTP response.
pub fn response(answer: &[u8]) -> Result<(u16, Vec<u8>), String> {
    let end = answer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or("Docker's answer has no end to its headers")?;
    let head = String::from_utf8_lossy(&answer[..end]);
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or("Docker's answer has no status")?;
    let chunked = lines.any(|line| {
        line.split_once(':')
            .is_some_and(|(name, value)| name.trim().eq_ignore_ascii_case("transfer-encoding") && value.trim().eq_ignore_ascii_case("chunked"))
    });
    let body = &answer[end + 4..];
    if !chunked {
        return Ok((status, body.to_vec()));
    }
    let mut joined = Vec::new();
    let mut at = 0;
    loop {
        let line_end = body[at..]
            .windows(2)
            .position(|window| window == b"\r\n")
            .ok_or("a chunk of Docker's answer has no size")?;
        let size = String::from_utf8_lossy(&body[at..at + line_end]);
        let size = usize::from_str_radix(size.split(';').next().unwrap_or("").trim(), 16).map_err(|_| "a chunk of Docker's answer has no size")?;
        at += line_end + 2;
        if size == 0 {
            return Ok((status, joined));
        }
        joined.extend_from_slice(body.get(at..at + size).ok_or("Docker's answer ends inside a chunk")?);
        at += size + 2;
    }
}

/// A stream to the engine.
trait Stream: Read + Write + Send {}
impl<T: Read + Write + Send> Stream for T {}

fn connect(endpoint: &Endpoint) -> Result<Box<dyn Stream>, String> {
    match endpoint {
        Endpoint::Pipe(name) => pipe(name),
        Endpoint::Socket(path) => socket(path),
        Endpoint::Tcp(address) => std::net::TcpStream::connect(address)
            .map(|stream| Box::new(stream) as Box<dyn Stream>)
            .map_err(|error| format!("Docker is not answering at {address}: {error}")),
    }
}

#[cfg(windows)]
fn pipe(name: &str) -> Result<Box<dyn Stream>, String> {
    const NOT_FOUND: i32 = 2;
    const DENIED: i32 = 5;
    const BUSY: i32 = 231;
    // A pipe whose every instance is taking another question is busy for a
    // moment; a few tries a little apart see one free.
    for _ in 0..10 {
        match std::fs::OpenOptions::new().read(true).write(true).open(name) {
            Ok(file) => return Ok(Box::new(file)),
            Err(error) if error.raw_os_error() == Some(BUSY) => std::thread::sleep(Duration::from_millis(100)),
            Err(error) if error.raw_os_error() == Some(NOT_FOUND) => return Err("Docker is not running".to_owned()),
            Err(error) if error.raw_os_error() == Some(DENIED) => {
                return Err("Docker refused to answer this account: it answers the members of docker-users".to_owned());
            }
            Err(error) => return Err(format!("Docker could not be reached at {name}: {error}")),
        }
    }
    Err("Docker's pipe stayed busy".to_owned())
}

#[cfg(not(windows))]
fn pipe(name: &str) -> Result<Box<dyn Stream>, String> {
    Err(format!("{name} is a Windows pipe"))
}

#[cfg(unix)]
fn socket(path: &str) -> Result<Box<dyn Stream>, String> {
    match std::os::unix::net::UnixStream::connect(path) {
        Ok(stream) => Ok(Box::new(stream)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Err("Docker is not running".to_owned()),
        Err(error) => Err(format!("Docker could not be reached at {path}: {error}")),
    }
}

#[cfg(not(unix))]
fn socket(path: &str) -> Result<Box<dyn Stream>, String> {
    Err(format!("{path} is a socket file, which this platform does not open"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An answer in the shape Docker 28 gives, every figure made up.
    const ANSWER: &str = r#"{
        "LayersSize": 10000,
        "Images": [
            {"Id": "a", "RepoTags": ["app:latest"], "Size": 6000, "SharedSize": 1000, "Containers": 1},
            {"Id": "b", "RepoTags": ["base:1"], "Size": 2500, "SharedSize": 1000, "Containers": 0},
            {"Id": "c", "RepoTags": [], "Size": 900, "SharedSize": 0, "Containers": 0},
            {"Id": "d", "RepoTags": ["<none>:<none>"], "Size": 700, "SharedSize": 200, "Containers": 0},
            {"Id": "e", "RepoTags": null, "Size": -1, "SharedSize": -1, "Containers": 1}
        ],
        "Containers": [
            {"Id": "1", "SizeRw": 300, "State": "running"},
            {"Id": "2", "SizeRw": 40, "State": "exited"},
            {"Id": "3", "SizeRw": 2, "State": "created"},
            {"Id": "4", "SizeRw": 5, "State": "paused"}
        ],
        "Volumes": [
            {"Name": "db", "UsageData": {"Size": 5000, "RefCount": 1}},
            {"Name": "old", "UsageData": {"Size": 800, "RefCount": 0}},
            {"Name": "unmeasured", "UsageData": {"Size": -1, "RefCount": 0}},
            {"Name": "bare"}
        ],
        "BuildCache": [
            {"ID": "x", "Size": 4000, "InUse": false, "Shared": false},
            {"ID": "y", "Size": 100, "InUse": true, "Shared": false},
            {"ID": "z", "Size": 9999, "InUse": false, "Shared": true}
        ]
    }"#;

    #[test]
    fn what_can_go_is_reckoned_as_docker_reckons_it() {
        let usage = parse(ANSWER.as_bytes()).expect("reads");
        // Of 10 000 bytes of layers, the image a container runs from holds
        // 5000 of its own; the rest is what no container needs.
        assert_eq!(
            usage.images,
            Part {
                count: 5,
                size: 10_000,
                idle: 3,
                reclaimable: 5000
            }
        );
        // An empty list, no list and only `<none>` are all no name.
        assert_eq!(
            usage.untagged,
            Part {
                count: 3,
                size: 900 + 500,
                idle: 2,
                reclaimable: 900 + 500
            }
        );
        assert_eq!(
            usage.containers,
            Part {
                count: 4,
                size: 347,
                idle: 2,
                reclaimable: 42
            },
            "paused is not stopped"
        );
        assert_eq!(
            usage.volumes,
            Part {
                count: 4,
                size: 5800,
                idle: 1,
                reclaimable: 800
            },
            "a volume Docker did not measure is counted, not sized"
        );
        assert_eq!(
            usage.build_cache,
            Part {
                count: 2,
                size: 4100,
                idle: 1,
                reclaimable: 4000
            },
            "a shared record is counted under the one it is shared with"
        );
        assert_eq!(usage.size(), 10_000 + 347 + 5800 + 4100);
    }

    #[test]
    fn an_answer_without_a_kind_has_none_of_it() {
        assert_eq!(parse(br#"{"LayersSize": 0}"#).expect("reads"), Usage::default());
        assert!(parse(b"<html>").is_err());
    }

    #[test]
    fn the_endpoint_is_docker_hosts_or_the_platforms_own() {
        let default = Endpoint::from(None, false).expect("a default");
        if cfg!(windows) {
            assert_eq!(default, Endpoint::Pipe(r"\\.\pipe\docker_engine".to_owned()));
        } else {
            assert_eq!(default, Endpoint::Socket("/var/run/docker.sock".to_owned()));
        }
        assert_eq!(Endpoint::from(Some("  "), false).expect("empty is unset"), default);
        assert_eq!(
            Endpoint::from(Some("npipe:////./pipe/dockerDesktopLinuxEngine"), false),
            Ok(Endpoint::Pipe(r"\\.\pipe\dockerDesktopLinuxEngine".to_owned()))
        );
        assert_eq!(
            Endpoint::from(Some("unix:///run/user/1000/docker.sock"), false),
            Ok(Endpoint::Socket("/run/user/1000/docker.sock".to_owned()))
        );
        assert_eq!(
            Endpoint::from(Some("tcp://127.0.0.1:2375"), false),
            Ok(Endpoint::Tcp("127.0.0.1:2375".to_owned()))
        );
        assert!(Endpoint::from(Some("tcp://127.0.0.1:2376"), true).expect_err("TLS").contains("TLS"));
        assert!(Endpoint::from(Some("ssh://me@box"), false).is_err());
    }

    #[test]
    fn a_response_is_read_whole_or_put_together_from_its_chunks() {
        let whole = b"HTTP/1.0 200 OK\r\nContent-Type: application/json\r\n\r\n{\"a\":1}";
        assert_eq!(response(whole), Ok((200, b"{\"a\":1}".to_vec())));
        let chunked = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\n{\"a\"\r\n3;x=y\r\n:1}\r\n0\r\n\r\n";
        assert_eq!(response(chunked), Ok((200, b"{\"a\":1}".to_vec())));
        assert_eq!(response(b"HTTP/1.0 400 Bad Request\r\n\r\ntoo new").map(|(status, _)| status), Ok(400));
        assert!(response(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n9\r\nshort").is_err());
        assert!(response(b"garbage").is_err());
    }

    #[test]
    fn a_question_is_asked_over_the_stream_and_its_answer_read() {
        // An engine on a loopback port: it reads the question and answers
        // with the shape above.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a port");
        let address = listener.local_addr().expect("its address").to_string();
        let engine = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("a question");
            let mut question = [0_u8; 1024];
            let read = stream.read(&mut question).expect("it reads");
            let question = String::from_utf8_lossy(&question[..read]).into_owned();
            let answer = format!("HTTP/1.0 200 OK\r\nContent-Type: application/json\r\n\r\n{ANSWER}");
            stream.write_all(answer.as_bytes()).expect("it answers");
            question
        });
        let usage = ask(&Endpoint::Tcp(address), Duration::from_secs(10)).expect("an answer");
        assert_eq!(usage.images.reclaimable, 5000);
        let question = engine.join().expect("the engine");
        assert!(question.starts_with("GET /v1.44/system/df HTTP/1.0\r\n"), "{question}");
    }

    #[test]
    fn an_engine_that_is_not_there_is_said_plainly() {
        let missing = if cfg!(windows) {
            Endpoint::Pipe(r"\\.\pipe\midda-no-such-engine".to_owned())
        } else {
            Endpoint::Socket("/nonexistent/midda-no-such-engine.sock".to_owned())
        };
        assert_eq!(ask(&missing, Duration::from_secs(10)), Err("Docker is not running".to_owned()));
    }
}
