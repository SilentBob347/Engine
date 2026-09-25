//! 测试合成内容：文本、图片与文件各一份，按单元、设备与步骤区分；不含任何真实用户资料。

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// 1x1 像素 PNG。
const PIXEL_PNG: [u8; 67] = [
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89, 0x00, 0x00, 0x00, 0x0a, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0d, 0x0a, 0x2d, 0xb4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae,
    0x42, 0x60, 0x82,
];

#[derive(Clone, Debug)]
pub(crate) enum Content {
    Text(String),
    Image(Vec<u8>),
    File {
        handle: String,
        name: String,
        bytes: Vec<u8>,
    },
}

pub(crate) fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(crate) fn content_digest(content: &Content) -> String {
    match content {
        Content::Text(text) => digest(text.as_bytes()),
        Content::Image(bytes) | Content::File { bytes, .. } => digest(bytes),
    }
}

/// 一组代表性内容；`tag` 使不同单元、设备与步骤的内容互不相同，捕获时间按步骤递增。
pub(crate) fn representative(tag: &str) -> Vec<Content> {
    // 图片在 PNG 尾部后追加标记字节，保持可解码的同时区分内容。
    let mut image = PIXEL_PNG.to_vec();
    image.extend_from_slice(tag.as_bytes());
    vec![
        Content::Text(format!("upgrade matrix synthetic text {tag}")),
        Content::Image(image),
        Content::File {
            handle: format!("file-{tag}"),
            name: format!("synthetic-{tag}.bin"),
            bytes: format!("upgrade matrix synthetic file {tag}").into_bytes(),
        },
    ]
}

pub(crate) fn text(tag: &str) -> Content {
    Content::Text(format!("upgrade matrix synthetic text {tag}"))
}

impl Content {
    pub(crate) fn capture_request(&self) -> Value {
        let observed_at_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| {
                i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
            });
        match self {
            Self::Text(text) => {
                json!({ "kind": "text", "text": text, "observed_at_ms": observed_at_ms })
            }
            Self::Image(bytes) => {
                json!({ "kind": "image", "bytes": bytes, "observed_at_ms": observed_at_ms })
            }
            Self::File {
                handle,
                name,
                bytes,
            } => json!({
                "kind": "file",
                "handle": handle,
                "display_name": name,
                "bytes": bytes,
                "observed_at_ms": observed_at_ms,
            }),
        }
    }
}
