//! 两台设备之间的公开流程：配对、双向发送文本与文件、查询设备组选择、移除与重新加入。
//! 只以公开查询判断收敛，不以固定等待代替。

use serde_json::{Value, json};
use uc_testkit::{FailureKind, ScenarioFailure};

use crate::{
    cell::CellRun,
    device::{Deadline, Device, failure},
    fixture::digest,
};

/// A 创建 Space（如尚未创建）并邀请 B 加入，等待双方都确认配对完成。
pub(crate) async fn pair(
    inviter: &mut Device,
    joiner: &mut Device,
    create: bool,
) -> Result<(), ScenarioFailure> {
    if create {
        inviter.create_space().await?;
    }
    let invitation = inviter.call("invite", Value::Null, "invite-failed").await?;
    let code = invitation["code"]
        .as_str()
        .ok_or_else(|| failure(FailureKind::ProductInvariant, "invitation-empty"))?
        .to_owned();
    joiner
        .call(
            "join",
            json!({ "invitation": code, "name": joiner.name }),
            "join-failed",
        )
        .await?;
    let deadline = Deadline::new("pairing-not-completed");
    loop {
        let setup = joiner.raw("setup", Value::Null).await?;
        if setup["ok"]["has_completed"] == true {
            break;
        }
        deadline.tick().await?;
    }
    inviter.id = Some(local_id(inviter).await?);
    joiner.id = Some(local_id(joiner).await?);
    let deadline = Deadline::new("paired-device-not-listed");
    loop {
        let a = inviter.observe().await?;
        let b = joiner.observe().await?;
        if lists(&a, joiner.name) && lists(&b, inviter.name) {
            return Ok(());
        }
        deadline.tick().await?;
    }
}

async fn local_id(device: &mut Device) -> Result<String, ScenarioFailure> {
    let observed = device.observe().await?;
    Device::local_id(&observed)
        .ok_or_else(|| failure(FailureKind::ProductInvariant, "local-device-id-missing"))
}

fn lists(observed: &Value, name: &str) -> bool {
    observed["devices"].as_array().is_some_and(|devices| {
        devices
            .iter()
            .any(|device| device["name"] == name && device["local"] == false)
    })
}

fn peer_id(device: &Device) -> Result<String, ScenarioFailure> {
    device
        .id
        .clone()
        .ok_or_else(|| failure(FailureKind::FixtureInvalid, "peer-id-unknown"))
}

/// 发送结果中各目标的结局，只保留结局种类，不含标识。
fn send_outcome(report: &Value) -> Value {
    json!(report["per_target"].as_array().map(|targets| {
        targets
            .iter()
            .map(|target| target["outcome"]["kind"].clone())
            .collect::<Vec<_>>()
    }))
}

/// 发送方向接收方发送一段文本与一个文件，等待接收方历史出现相同内容。
async fn send_both_kinds(
    run: &mut CellRun,
    sender: &mut Device,
    receiver: &mut Device,
    tag: &str,
) -> Result<(), ScenarioFailure> {
    let peer = peer_id(receiver)?;
    let text = format!("upgrade matrix synthetic message {tag}");
    // 旧版的 `connected` 只反映当前是否已有连接，发送时才建立；因此只要求对端已配对，
    // 然后发送一次并以接收方收到为准，不重复发送。
    let deadline = Deadline::new("peer-not-paired");
    loop {
        let peers = sender
            .call("peers", Value::Null, "peers-query-failed")
            .await?;
        let paired = peers.as_array().is_some_and(|peers| {
            peers
                .iter()
                .any(|entry| entry["peer_id"] == peer.as_str() && entry["is_paired"] == true)
        });
        if paired {
            break;
        }
        deadline.tick().await?;
    }
    let report = sender
        .call(
            "send",
            json!({ "text": text, "peer": peer }),
            "send-text-failed",
        )
        .await?;
    run.fact(&format!("send-text-{tag}"), send_outcome(&report));
    receiver
        .wait_for_content(&digest(text.as_bytes()), "sent-text-not-received")
        .await?;
    let file = format!("upgrade matrix synthetic shared file {tag}");
    let report = sender
        .call(
            "send_file",
            json!({
                "handle": format!("shared-{tag}"),
                "display_name": format!("shared-{tag}.bin"),
                "content": file,
                "peer": peer,
            }),
            "send-file-failed",
        )
        .await?;
    run.fact(&format!("send-file-{tag}"), send_outcome(&report));
    receiver
        .wait_for_content(&digest(file.as_bytes()), "sent-file-not-received")
        .await?;
    Ok(())
}

/// 双向互通：两个方向各发送文本与文件，并在支持的版本上查询设备组选择。
pub(crate) async fn exchange(
    run: &mut CellRun,
    a: &mut Device,
    b: &mut Device,
    tag: &str,
) -> Result<(), ScenarioFailure> {
    send_both_kinds(run, a, b, &format!("{tag}-ab")).await?;
    send_both_kinds(run, b, a, &format!("{tag}-ba")).await?;
    for device in [a, b] {
        let reply = device.raw("eligibility", Value::Null).await?;
        let key = format!("device-group-choices-{}-{tag}", device.label);
        if reply.get("ok").is_some() {
            run.fact(&key, json!("queried"));
        } else if device.supports("device-group-choices") {
            return Err(failure(
                FailureKind::ProductInvariant,
                "device-group-choices-failed",
            ));
        } else {
            run.fact(&key, json!("not-supported-by-version"));
        }
    }
    Ok(())
}

/// A 移除 B，等待 A 的设备列表不再包含 B；被移除方看到的状态随版本不同，只记录不断言。
pub(crate) async fn remove(
    run: &mut CellRun,
    remover: &mut Device,
    removed: &mut Device,
) -> Result<(), ScenarioFailure> {
    let peer = peer_id(removed)?;
    remover
        .call("remove", json!({ "peer": peer }), "remove-member-failed")
        .await?;
    let deadline = Deadline::new("removal-not-converged");
    loop {
        let observed = remover.observe().await?;
        if !lists(&observed, removed.name) {
            break;
        }
        deadline.tick().await?;
    }
    let seen = removed.observe().await?;
    run.fact(
        &format!("removed-device-view-{}", removed.label),
        json!({ "still_lists_remover": lists(&seen, remover.name), "setup": seen["setup"] }),
    );
    Ok(())
}

/// 被移除的设备以新邀请重新加入并恢复双向互通。
pub(crate) async fn remove_and_rejoin(
    run: &mut CellRun,
    inviter: &mut Device,
    joiner: &mut Device,
    tag: &str,
) -> Result<(), ScenarioFailure> {
    remove(run, inviter, joiner).await?;
    pair(inviter, joiner, false).await?;
    exchange(run, inviter, joiner, tag).await
}
