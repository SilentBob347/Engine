//! D2 两台设备先后升级：两台旧版设备已配对并有历史；A 先升级并与仍为旧版的 B 互通，B 再升级。

use serde_json::json;
use uc_testkit::{FailureKind, ScenarioFailure};

use crate::{
    catalog::Point,
    cell::CellRun,
    device::{Device, failure, write_representative_content},
    dimensions::upgrade::upgrade_and_verify,
    fixture::representative,
    interop::{exchange, pair as pair_devices, remove, remove_and_rejoin},
};

async fn upgrade_one(
    run: &mut CellRun,
    device: &mut Device,
    to: &Point,
) -> Result<(), ScenarioFailure> {
    let before = device.unlocked_observation().await?;
    device.stop(run).await?;
    upgrade_and_verify(run, device, to, &before).await?;
    Ok(())
}

pub(crate) async fn pair(
    run: &mut CellRun,
    from: &Point,
    to: &Point,
) -> Result<(), ScenarioFailure> {
    let mut a = Device::new(run, "a", "Device A")?;
    let mut b = Device::new(run, "b", "Device B")?;
    {
        let _stage = run.stage("old-write");
        a.start(run, from).await?;
        b.start(run, from).await?;
        pair_devices(run, &mut a, &mut b, true).await?;
        write_representative_content(&mut a, &representative("a-0")).await?;
        exchange(run, &mut a, &mut b, "old").await?;
    }
    upgrade_one(run, &mut a, to).await?;
    {
        let _stage = run.stage("interop");
        exchange(run, &mut a, &mut b, "mixed").await?;
    }
    upgrade_one(run, &mut b, to).await?;
    {
        let _stage = run.stage("continue");
        exchange(run, &mut a, &mut b, "new").await?;
        // 移除后重新加入在双方都升级后验证：旧版自身的重新加入缺陷不应遮住升级兼容结果。
        remove_and_rejoin(run, &mut a, &mut b, "new-rejoined").await?;
        remove(run, &mut a, &mut b).await?;
    }
    let _stage = run.stage("cleanup");
    a.stop(run).await?;
    b.stop(run).await
}

/// 完整链：两台设备同步逐级走完所有版本点，每级先升 A、互通，再升 B、互通。
pub(crate) async fn chain(
    run: &mut CellRun,
    points: &[Point],
    excluded: Vec<String>,
) -> Result<(), ScenarioFailure> {
    let mut a = Device::new(run, "a", "Device A")?;
    let mut b = Device::new(run, "b", "Device B")?;
    let mut skipped = excluded;
    let mut started = false;
    for point in points {
        run.fact("current_point", json!(point.id));
        if !started {
            let _stage = run.stage("old-write");
            match a.start(run, point).await {
                Err(error) if error.kind() == FailureKind::EnvironmentUnavailable => {
                    skipped.push(point.id.clone());
                    run.fact("skipped_points", json!(skipped));
                    continue;
                }
                result => {
                    result?;
                }
            }
            b.start(run, point).await?;
            pair_devices(run, &mut a, &mut b, true).await?;
            write_representative_content(&mut a, &representative("a-0")).await?;
            exchange(run, &mut a, &mut b, &format!("{}-start", point.id)).await?;
            started = true;
            continue;
        }
        match upgrade_one(run, &mut a, point).await {
            Err(error) if error.kind() == FailureKind::EnvironmentUnavailable => {
                return Err(failure(
                    FailureKind::ProductInvariant,
                    "chain-point-became-unavailable",
                ));
            }
            result => result?,
        }
        {
            let _stage = run.stage("interop");
            exchange(run, &mut a, &mut b, &format!("{}-mixed", point.id)).await?;
        }
        upgrade_one(run, &mut b, point).await?;
        let _stage = run.stage("interop");
        exchange(run, &mut a, &mut b, &format!("{}-new", point.id)).await?;
    }
    if !started {
        return Err(failure(
            FailureKind::EnvironmentUnavailable,
            "no-runnable-point",
        ));
    }
    run.fact("skipped_points", json!(skipped));
    {
        let _stage = run.stage("continue");
        remove_and_rejoin(run, &mut a, &mut b, "final-rejoined").await?;
        remove(run, &mut a, &mut b).await?;
    }
    let _stage = run.stage("cleanup");
    a.stop(run).await?;
    b.stop(run).await
}
