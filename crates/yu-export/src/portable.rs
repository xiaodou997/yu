//! Frozen local resources and destination transactions for document export.
//! Temporary file creation/publication uses tempfile, not a custom temp scheme.
use crate::document::{ExportImage, MAX_OUTPUT_BYTES, ResourceError};
use base64::Engine;
use std::collections::HashMap;
use std::fs::{self, File, Metadata};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub const MAX_RESOURCE_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_RESOURCE_TOTAL: usize = 128 * 1024 * 1024;
pub const MAX_IMAGE_PIXELS: u64 = 32 * 1024 * 1024;
#[derive(Clone, Debug, PartialEq, Eq)]
struct Stamp {
    length: u64,
    modified: Option<SystemTime>,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(unix)]
    change: (i64, i64),
}
impl Stamp {
    fn from(meta: &Metadata) -> Self {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        Self {
            length: meta.len(),
            modified: meta.modified().ok(),
            #[cfg(unix)]
            device: meta.dev(),
            #[cfg(unix)]
            inode: meta.ino(),
            #[cfg(unix)]
            change: (meta.ctime(), meta.ctime_nsec()),
        }
    }
    fn same_file(&self, other: &Self) -> bool {
        #[cfg(unix)]
        {
            self.device == other.device && self.inode == other.inode
        }
        #[cfg(not(unix))]
        {
            self == other
        }
    }
}
#[derive(Clone, Debug)]
pub struct ProtectedFile {
    path: PathBuf,
    stamp: Option<Stamp>,
}
impl ProtectedFile {
    pub fn capture(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
            stamp: fs::metadata(path).ok().map(|meta| Stamp::from(&meta)),
        }
    }
}
pub struct Destination {
    path: PathBuf,
    initial: Option<Stamp>,
    parent: PathBuf,
    parent_identity: Stamp,
}
impl Destination {
    /// Call at destination confirmation, before long-running preparation.
    pub fn capture(path: &Path, replace_existing: bool) -> Result<Self, String> {
        let parent = path
            .parent()
            .ok_or("导出目标没有父目录")?
            .canonicalize()
            .map_err(|_| "目标目录不可访问")?;
        let name = path.file_name().ok_or("导出文件名无效")?;
        let path = parent.join(name);
        let initial = match fs::symlink_metadata(&path) {
            Ok(meta) => {
                if !replace_existing {
                    return Err("目标已存在，请明确确认覆盖".into());
                }
                if !meta.is_file() || meta.file_type().is_symlink() {
                    return Err("不能替换目录或符号链接目标".into());
                }
                Some(Stamp::from(&meta))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => return Err("无法检查目标文件".into()),
        };
        let parent_identity = Stamp::from(&fs::metadata(&parent).map_err(|_| "无法检查目标目录")?);
        Ok(Self {
            path,
            initial,
            parent,
            parent_identity,
        })
    }
    pub fn validate(&self, protected: &[ProtectedFile]) -> Result<(), String> {
        let parent = self.path.parent().ok_or("目标目录丢失")?;
        let parent_meta = fs::metadata(parent).map_err(|_| "目标目录已不可访问")?;
        if !self.parent_identity.same_file(&Stamp::from(&parent_meta)) {
            return Err("导出过程中目标目录已改变".into());
        }
        let current = match fs::symlink_metadata(&self.path) {
            Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() => {
                Some(Stamp::from(&meta))
            }
            Ok(_) => return Err("导出过程中目标变成非普通文件".into()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => return Err("无法复核目标文件".into()),
        };
        if current != self.initial {
            return Err("导出过程中目标已改变，请重新确认".into());
        }
        for file in protected {
            let same_path = file.path == self.path
                || file.path.canonicalize().ok().as_ref() == Some(&self.path);
            let now = fs::metadata(&file.path).ok().map(|meta| Stamp::from(&meta));
            let alias = current.as_ref().is_some_and(|target| {
                file.stamp
                    .as_ref()
                    .into_iter()
                    .chain(now.as_ref())
                    .any(|stamp| target.same_file(stamp))
            });
            if same_path || alias {
                return Err("导出目标不能覆盖源文档、引用图片或其文件别名".into());
            }
        }
        Ok(())
    }
    pub fn publish(
        self,
        bytes: &[u8],
        protected: &[ProtectedFile],
        mut checkpoint: impl FnMut() -> Result<(), String>,
    ) -> Result<(), String> {
        self.publish_guarded(bytes, protected, &mut checkpoint, || Ok(()))
    }
    /// Hold the owner's commit guard across the atomic publication. Cancellation
    /// that wins before the guard rejects publication; after it, commit wins.
    pub fn publish_guarded<G>(
        self,
        bytes: &[u8],
        protected: &[ProtectedFile],
        mut checkpoint: impl FnMut() -> Result<(), String>,
        begin_commit: impl FnOnce() -> Result<G, String>,
    ) -> Result<(), String> {
        if bytes.len() > MAX_OUTPUT_BYTES {
            return Err("输出超过预算".into());
        }
        self.validate(protected)?;
        checkpoint()?;
        let mut temporary = tempfile::Builder::new()
            .prefix(".yu-export-")
            .tempfile_in(&self.parent)
            .map_err(|_| "不能在目标目录创建导出临时文件")?;
        for part in bytes.chunks(1024 * 1024) {
            checkpoint()?;
            temporary
                .write_all(part)
                .map_err(|_| "写入导出临时文件失败")?;
        }
        temporary
            .as_file()
            .sync_all()
            .map_err(|_| "同步导出临时文件失败")?;
        self.validate(protected)?;
        checkpoint()?;
        if self.initial.is_some() {
            let _commit_guard = begin_commit()?;
            temporary
                .persist(&self.path)
                .map_err(|_| "提交导出文件失败；旧目标保持不变")?;
        } else {
            let _commit_guard = begin_commit()?;
            temporary
                .persist_noclobber(&self.path)
                .map_err(|_| "目标已出现或无法提交；没有覆盖已有文件")?;
        }
        // Publication succeeded. Do not report a post-rename cancellation as
        // failure: the commit point is explicit and no rollback deletes output.
        Ok(())
    }
}
struct FrozenImage {
    path: PathBuf,
    stamp: Stamp,
    bytes: Vec<u8>,
}
pub struct FrozenImages {
    document: Option<PathBuf>,
    files: HashMap<String, Result<FrozenImage, String>>,
    total: usize,
    pub protected: Vec<ProtectedFile>,
}
impl FrozenImages {
    pub fn new(document: Option<PathBuf>, source: Option<&Path>) -> Self {
        Self {
            document,
            files: HashMap::new(),
            total: 0,
            protected: source.map(ProtectedFile::capture).into_iter().collect(),
        }
    }
    pub fn capture(&mut self, destination: &str) -> Result<(), ResourceError> {
        if self.files.contains_key(destination) {
            return Ok(());
        }
        let lower = destination.trim().to_ascii_lowercase();
        if lower.starts_with("//")
            || lower.contains("://")
            || lower.starts_with("data:")
            || lower.contains('\0')
        {
            self.files.insert(
                destination.to_owned(),
                Err("远程或内嵌图片未自动加载".into()),
            );
            return Ok(());
        }
        if self.document.is_none() && !Path::new(destination).is_absolute() {
            return Err(ResourceError::Fatal(
                "未命名文档的相对图片需要选择资源基准目录".into(),
            ));
        }
        let basis = self
            .document
            .as_deref()
            .unwrap_or(Path::new("/untitled.md"));
        let path = match yu_assets::ImageLocation::resolve(basis, destination) {
            Ok(location) => location.path().to_path_buf(),
            Err(_) => {
                self.files
                    .insert(destination.to_owned(), Err("图片路径无法解析".into()));
                return Ok(());
            }
        };
        self.protected.push(ProtectedFile::capture(&path));
        let read = (|| {
            let meta = fs::metadata(&path).map_err(|_| "图片不存在或无读取权限")?;
            if !meta.is_file() {
                return Err("图片不是普通文件");
            }
            if meta.len() > MAX_RESOURCE_BYTES as u64 {
                return Err("图片超过 32 MiB 资源预算");
            }
            let mut file = File::open(&path).map_err(|_| "图片无法读取")?;
            let before = Stamp::from(&file.metadata().map_err(|_| "图片身份不可读取")?);
            let mut bytes = Vec::new();
            Read::by_ref(&mut file)
                .take(MAX_RESOURCE_BYTES as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| "图片读取失败")?;
            let after = Stamp::from(&file.metadata().map_err(|_| "图片身份不可读取")?);
            if before != after || before != Stamp::from(&meta) || bytes.len() > MAX_RESOURCE_BYTES {
                return Err("图片在准备过程中改变或超限");
            }
            Ok(FrozenImage {
                path,
                stamp: before,
                bytes,
            })
        })();
        match read {
            Ok(image) => {
                self.total = self
                    .total
                    .checked_add(image.bytes.len())
                    .ok_or_else(|| ResourceError::Fatal("图片总字节溢出".into()))?;
                if self.total > MAX_RESOURCE_TOTAL {
                    return Err(ResourceError::Fatal("图片总量超过 128 MiB 预算".into()));
                }
                self.files.insert(destination.to_owned(), Ok(image));
            }
            Err(message)
                if message.contains("预算")
                    || message.contains("改变")
                    || message.contains("超限") =>
            {
                return Err(ResourceError::Fatal(message.into()));
            }
            Err(message) => {
                self.files
                    .insert(destination.to_owned(), Err(message.into()));
            }
        }
        Ok(())
    }
    pub fn bytes(&self, destination: &str) -> Result<&[u8], ResourceError> {
        match self.files.get(destination) {
            Some(Ok(image)) => Ok(&image.bytes),
            Some(Err(message)) => Err(ResourceError::Warning(message.clone())),
            None => Err(ResourceError::Fatal("图片未纳入固定资源快照".into())),
        }
    }
    pub fn verify(&self) -> Result<(), String> {
        for image in self
            .files
            .values()
            .filter_map(|result| result.as_ref().ok())
        {
            let current = fs::metadata(&image.path).map_err(|_| "准备过程中图片消失")?;
            if Stamp::from(&current) != image.stamp {
                return Err("准备过程中图片改变，请重新导出".into());
            }
        }
        Ok(())
    }
}
pub fn data_image(bytes: &[u8], mime: &str, width: u32, height: u32) -> ExportImage {
    ExportImage {
        data_uri: format!(
            "data:{mime};base64,{}",
            base64::engine::general_purpose::STANDARD.encode(bytes)
        ),
        width,
        height,
        baseline_milli: None,
    }
}
/// Validate an SVG image before embedding it in its own image document. DTDs,
/// active elements, event attributes and external references are not permitted.
/// This uses the already-locked XML parser, not a handwritten SVG interpreter.
pub fn validate_svg(markup: &str) -> Result<(), String> {
    if markup.len() > yu_assets::EMBEDDED_SVG_MAX_MARKUP_BYTES {
        return Err("SVG 超过 4 MiB 预算".into());
    }
    let document = roxmltree::Document::parse_with_options(
        markup,
        roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: 100_000,
            ..Default::default()
        },
    )
    .map_err(|_| "SVG XML 无效或超过节点预算")?;
    if document.root_element().tag_name().name() != "svg" {
        return Err("资源不是 SVG".into());
    }
    if document.descendants().any(|node| node.pi().is_some()) {
        return Err("SVG 包含未支持的处理指令".into());
    }
    for node in document.descendants().filter(|node| node.is_element()) {
        let name = node.tag_name().name().to_ascii_lowercase();
        if matches!(
            name.as_str(),
            "script"
                | "foreignobject"
                | "animate"
                | "animatetransform"
                | "animatemotion"
                | "set"
                | "iframe"
                | "audio"
                | "video"
        ) {
            return Err("SVG 包含主动内容".into());
        }
        for attr in node.attributes() {
            let name = attr.name().to_ascii_lowercase();
            let value = attr.value().trim();
            if name.starts_with("on")
                || (matches!(name.as_str(), "href" | "src") && !value.starts_with('#'))
            {
                return Err("SVG 包含脚本属性或外部资源引用".into());
            }
            check_svg_css(value)?;
        }
        if name == "style" {
            check_svg_css(node.text().unwrap_or(""))?;
        }
    }
    Ok(())
}
fn check_svg_css(value: &str) -> Result<(), String> {
    let lower = value.to_ascii_lowercase();
    // CSS escapes/comments can conceal loaders. The static helper does not
    // emit them; unsupported user styles fail visibly rather than being stripped.
    if lower.contains(['\\', '@']) || lower.contains("/*") || lower.contains("expression(") {
        return Err("SVG 样式包含未支持的主动或转义语法".into());
    }
    let mut rest = lower.as_str();
    while let Some(index) = rest.find("url(") {
        rest = &rest[index + 4..];
        let end = rest.find(')').ok_or("SVG 样式 URL 无效")?;
        if !rest[..end]
            .trim()
            .trim_matches(['\'', '"'])
            .starts_with('#')
        {
            return Err("SVG 样式引用外部资源".into());
        }
        rest = &rest[end + 1..];
    }
    Ok(())
}
