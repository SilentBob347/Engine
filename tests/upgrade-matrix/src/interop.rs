//! 两台设备之间的公开流程：配对、双向发送文本与文件、查询设备组选择、移除与重新加入。
//! 只以公开查询判断收敛，不以固定等待代替。

use serde_json::{Value, json};
use uc_testkit::{FailureKind, ScenarioFailure};

use crate::{
    cell::CellRun,
    device::{Deadline, Device, failure},
    fixture::digest,
};

/// 公开查询在运行期重组或设置未完成时返回的可重试错误码：操作暂不可用（1103）、尚未完成设置（1211）。
const NOT_YET_AVAILABLE_CODES: [u64; 2] = [1103, 1211];

/// 邀请对账尚未完成（移除成员后的公开过渡状态）；旧版以该错误码拒绝新邀请，稍后可再邀请。
const INVITATION_RECONCILIATION_PENDING_CODE: u64 = 1225;

/// 发出邀请；遇到对账尚未完成时在产品期限内等待其结束，并记录等待次数，其余错误立即失败。
async fn invite(run: &mut CellRun, inviter: &mut Device) -> Result<Value, ScenarioFailure> {
    let deadline = Deadline::new("invitation-reconciliation-pending");
    let mut pending = 0_u32;
    loop {
        let reply = inviter.raw("invite", Value::Null).await?;
        if let Some(invitation) = reply.get("ok") {
            if pending > 0 {
                run.fact(
                    &format!("invitation-reconciliation-pending-{}", inviter.label),
                    json!(pending),
                );
            }
            return Ok(invitation.clone());
        }
        if reply["code"].as_u64() != Some(INVITATION_RECONCILIATION_PENDING_CODE) {
            inviter.record_error("invite", &reply);
            return Err(failure(FailureKind::ProductInvariant, "invite-failed"));
        }
        pending += 1;
        deadline.tick().await?;
    }
}

/// A 创建 Space（如尚未创建）并邀请 B 加入，等待双方都确认配对完成。
pub(crate) async fn pair(
    run: &mut CellRun,
    inviter: &mut Device,
    joiner: &mut Device,
    create: bool,
) -> Result<(), ScenarioFailure> {
    if create {
        inviter.create_space().await?;
    }
    let invitation = invite(run, inviter).await?;
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

/// 一台设备的公开设备信任快照是否已稳定并把对端视为可同步：对端关系为 `usable`，没有进行中的
/// 成员变更或待处理的入站成员，加入记录为空或已是 `active`；当前源码另要求设备更新已完成。
fn unsettled_reasons(choices: &Value, peer: &str) -> Vec<Value> {
    let trust = &choices["device_trust"];
    let mut reasons = Vec::new();
    // 移除后重新加入时，同一设备可能同时保留一条已移除记录和一条新的可用关系。
    let relationships: Vec<Value> = trust["devices"]
        .as_array()
        .map(|devices| {
            devices
                .iter()
                .filter(|device| device["device_id"] == peer)
                .map(|device| device["sync_relationship"].clone())
                .collect()
        })
        .unwrap_or_default();
    if relationships.is_empty() {
        reasons.push(json!("peer_not_listed"));
    } else if !relationships
        .iter()
        .any(|relationship| relationship == "usable")
    {
        reasons.push(json!({ "peer_relationship": relationships }));
    }
    if !trust["current_change"].is_null() {
        reasons.push(json!({ "current_change": trust["current_change"]["kind"].clone() }));
    }
    if !(trust["current_join"].is_null() || trust["current_join"]["status"] == "active") {
        reasons.push(json!({ "current_join": trust["current_join"]["status"].clone() }));
    }
    if !trust["pending_inbound_member"].is_null() {
        reasons.push(json!("pending_inbound_member"));
    }
    if let Some(update) = trust.get("space_device_update")
        && update["phase"] != "completed"
    {
        reasons.push(json!({ "space_device_update": update["phase"].clone() }));
    }
    reasons
}

async fn peer_listed_as_paired(device: &mut Device, peer: &str) -> Result<bool, ScenarioFailure> {
    let peers = device
        .call("peers", Value::Null, "peers-query-failed")
        .await?;
    Ok(peers.as_array().is_some_and(|peers| {
        peers
            .iter()
            .any(|entry| entry["peer_id"] == peer && entry["is_paired"] == true)
    }))
}

/// 一侧尚未公开确认可与对端同步的原因；为空表示已稳定。对端关系为 `usable`、没有进行中的成员变更或
/// 待处理的入站成员、加入记录为空或已是 `active`，当前源码另要求设备更新已完成；不支持设备组选择的
/// 早期版本只能以对端已配对为准。
async fn unsettled_for(
    device: &mut Device,
    peer: &str,
    notes: &mut Vec<Value>,
) -> Result<Vec<Value>, ScenarioFailure> {
    if device.supports("device-group-choices") {
        let reply = device.raw("eligibility", Value::Null).await?;
        let Some(choices) = reply.get("ok") else {
            // 运行期重组中（例如重新加入切换 Space）公开返回“暂不可用、可重试”或“尚未完成设置”，属于未稳定。
            if let Some(code) = reply["code"]
                .as_u64()
                .filter(|code| NOT_YET_AVAILABLE_CODES.contains(code))
            {
                return Ok(vec![json!({ "query_unavailable": code })]);
            }
            device.record_error("eligibility", &reply);
            return Err(failure(
                FailureKind::ProductInvariant,
                "device-group-choices-failed",
            ));
        };
        let mut reasons = unsettled_reasons(choices, peer);
        // 部分旧版在移除后重新加入时快照仍把对端标为已移除，但对端已配对且可收发；
        // 此时以已配对为准继续，并把不一致记录为事实，不隐藏。
        if let Some(first) = reasons.first()
            && first.get("peer_relationship").is_some()
            && peer_listed_as_paired(device, peer).await?
        {
            let relationship = reasons.remove(0);
            if reasons.is_empty() {
                notes.push(json!({ "device": device.label, "paired_but": relationship }));
            } else {
                reasons.insert(0, relationship);
            }
        }
        return Ok(reasons);
    }
    Ok(if peer_listed_as_paired(device, peer).await? {
        Vec::new()
    } else {
        vec![json!("peer_not_paired")]
    })
}

/// 发送前等待双方都公开确认成员关系已稳定：只看发送方会在接收方仍在处理成员更新时发送，
/// 旧版接收方会拒收且不重发。旧版的 `connected` 只在发送时建立，不作为前提。
/// 之后只发送一次并以接收方收到为准，不重复发送。
async fn wait_until_sendable(
    run: &mut CellRun,
    sender: &mut Device,
    receiver: &mut Device,
) -> Result<(), ScenarioFailure> {
    let sender_peer = peer_id(receiver)?;
    let receiver_peer = peer_id(sender)?;
    let deadline = Deadline::new("peers-not-settled");
    loop {
        let mut notes = Vec::new();
        let sender_reasons = unsettled_for(sender, &sender_peer, &mut notes).await?;
        let receiver_reasons = unsettled_for(receiver, &receiver_peer, &mut notes).await?;
        if sender_reasons.is_empty() && receiver_reasons.is_empty() {
            if !notes.is_empty() {
                run.fact("relationship-not-usable-while-paired", Value::Array(notes));
            }
            return Ok(());
        }
        if let Err(timeout) = deadline.tick().await {
            run.fact(
                "peers-not-settled",
                json!({ sender.label: sender_reasons, receiver.label: receiver_reasons }),
            );
            return Err(timeout);
        }
    }
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
    wait_until_sendable(run, sender, receiver).await?;
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
    pair(run, inviter, joiner, false).await?;
    exchange(run, inviter, joiner, tag).await
}
