//! 测试合成剪贴板内容：宿主按指令放入一份快照，再经公开 `CaptureCurrentClipboard` 写入历史。

use std::sync::{Arc, Mutex};

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use uc_engine::{
    Engine, HostCapabilityError, HostClipboard, HostClipboardRepresentation, HostClipboardSnapshot,
    HostFileHandle, Operation, OperationResult,
};

use crate::{string, unavailable, Files, ManagedFile};

#[derive(Clone)]
pub(crate) struct Clipboard(Arc<Mutex<HostClipboardSnapshot>>);

impl Default for Clipboard {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(HostClipboardSnapshot {
            observed_at_ms: 0,
            representations: vec![],
        })))
    }
}

impl HostClipboard for Clipboard {
    fn read(&self) -> Result<HostClipboardSnapshot, HostCapabilityError> {
        Ok(self.0.lock().map_err(|_| unavailable())?.clone())
    }
    fn write(&self, _: HostClipboardSnapshot) -> Result<(), HostCapabilityError> {
        Ok(())
    }
}

fn bytes(request: &Value) -> Result<Vec<u8>> {
    request["bytes"]
        .as_array()
        .context("missing test content bytes")?
        .iter()
        .map(|byte| {
            byte.as_u64()
                .and_then(|byte| u8::try_from(byte).ok())
                .context("invalid test content byte")
        })
        .collect()
}

/// 把一份合成内容放入测试剪贴板并捕获为历史条目。
pub(crate) async fn capture(
    engine: &Engine,
    clipboard: &Clipboard,
    files: &Files,
    request: &Value,
) -> Result<Value> {
    let observed_at_ms = request["observed_at_ms"]
        .as_i64()
        .context("missing capture time")?;
    let representation = match string(request, "kind")? {
        "text" => HostClipboardRepresentation::Inline {
            format: "text".into(),
            mime_type: Some("text/plain".into()),
            bytes: string(request, "text")?.as_bytes().to_vec(),
        },
        "image" => HostClipboardRepresentation::Inline {
            format: "image".into(),
            mime_type: Some("image/png".into()),
            bytes: bytes(request)?,
        },
        "file" => {
            let handle = string(request, "handle")?;
            let content = bytes(request)?;
            let size_bytes = content.len() as u64;
            let display_name = string(request, "display_name")?.to_owned();
            files.0.lock().map_err(|_| unavailable())?.insert(
                handle.to_owned(),
                ManagedFile {
                    display_name: display_name.clone(),
                    mime_type: Some("application/octet-stream".into()),
                    bytes: content,
                },
            );
            HostClipboardRepresentation::File {
                format: "files".into(),
                handle: HostFileHandle::new(handle),
                display_name,
                mime_type: Some("application/octet-stream".into()),
                size_bytes,
            }
        }
        _ => bail!("unknown test content kind"),
    };
    *clipboard.0.lock().map_err(|_| unavailable())? = HostClipboardSnapshot {
        observed_at_ms,
        representations: vec![representation],
    };
    match engine.execute(Operation::CaptureCurrentClipboard).await? {
        OperationResult::ClipboardCaptured { entry_id, .. } => Ok(json!({ "entry": entry_id })),
        _ => bail!("clipboard capture result expected"),
    }
}
