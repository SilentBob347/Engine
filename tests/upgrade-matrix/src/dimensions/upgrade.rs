//! D1 单设备资料升级：旧版建立资料并写入代表性内容后退出，新版打开同一资料目录核对并继续使用。

use serde_json::{Value, json};
use uc_testkit::{FailureKind, ScenarioFailure};

use crate::{
    catalog::Point,
    cell::CellRun,
    device::{Device, failure, write_representative_content},
    dimensions::require_preserved,
    fixture::{content_digest, representative, text},
};

pub(crate) async fn pair(
    run: &mut CellRun,
    from: &Point,
    to: &Point,
) -> Result<(), ScenarioFailure> {
    let mut device = Device::new(run, "a", "Device A")?;
    let before = {
        let _stage = run.stage("old-write");
        device.start(run, from).await?;
        device.create_space().await?;
        write_representative_content(&mut device, &representative("a-0")).await?;
        let observed = device.unlocked_observation().await?;
        device.stop(run).await?;
        observed
    };
    upgrade_and_verify(run, &mut device, to, &before).await?;
    {
        let _stage = run.stage("continue");
        continue_using(&mut device, "a-1").await?;
    }
    let _stage = run.stage("cleanup");
    device.stop(run).await
}

/// 打开新版并核对资料保留；返回升级后的观察。
pub(crate) async fn upgrade_and_verify(
    run: &mut CellRun,
    device: &mut Device,
    to: &Point,
    before: &Value,
) -> Result<Value, ScenarioFailure> {
    {
        let _stage = run.stage("upgrade");
        let startup = device.start(run, to).await?;
        run.fact(&format!("startup-{}-{}", device.label, to.id), startup);
    }
    let _stage = run.stage("verify");
    let after = device.unlocked_observation().await?;
    run.fact(
        &format!("upgrade-status-{}-{}", device.label, to.id),
        after["upgrade"].clone(),
    );
    require_preserved(run, before, &after, "state-changed-by-upgrade")?;
    Ok(after)
}

/// 升级后继续写入新内容并发出邀请，证明资料仍可正常使用。
pub(crate) async fn continue_using(
    device: &mut Device,
    tag: &str,
) -> Result<Value, ScenarioFailure> {
    let content = text(tag);
    device.capture(&content).await?;
    let observed = device
        .wait_for_content(&content_digest(&content), "new-content-missing")
        .await?;
    let invitation = device
        .call("invite", Value::Null, "invite-after-upgrade-failed")
        .await?;
    if invitation["code"].as_str().is_none_or(str::is_empty) {
        return Err(failure(FailureKind::ProductInvariant, "invitation-empty"));
    }
    Ok(observed)
}

/// 完整链：同一资料依次由每个版本打开；无法在本地网络运行的版本记入跳过点。
pub(crate) async fn chain(run: &mut CellRun, points: &[Point]) -> Result<(), ScenarioFailure> {
    let mut device = Device::new(run, "a", "Device A")?;
    let mut skipped = Vec::new();
    let mut previous: Option<Value> = None;
    for (step, point) in points.iter().enumerate() {
        run.fact("current_point", json!(point.id));
        match &previous {
            None => {
                let _stage = run.stage("old-write");
                match device.start(run, point).await {
                    Err(error) if error.kind() == FailureKind::EnvironmentUnavailable => {
                        skipped.push(point.id.clone());
                        run.fact("skipped_points", json!(skipped));
                        continue;
                    }
                    result => {
                        result?;
                    }
                }
                device.create_space().await?;
                write_representative_content(&mut device, &representative("a-0")).await?;
                previous = Some(device.unlocked_observation().await?);
            }
            Some(before) => {
                let before = before.clone();
                match upgrade_and_verify(run, &mut device, point, &before).await {
                    Err(error) if error.kind() == FailureKind::EnvironmentUnavailable => {
                        skipped.push(point.id.clone());
                        run.fact("skipped_points", json!(skipped));
                        continue;
                    }
                    result => {
                        result?;
                    }
                }
                let _stage = run.stage("continue");
                previous = Some(continue_using(&mut device, &format!("a-{step}")).await?);
            }
        }
        device.stop(run).await?;
    }
    if previous.is_none() {
        return Err(failure(
            FailureKind::EnvironmentUnavailable,
            "no-runnable-point",
        ));
    }
    run.fact("skipped_points", json!(skipped));
    Ok(())
}
