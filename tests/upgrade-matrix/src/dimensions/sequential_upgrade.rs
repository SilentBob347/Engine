//! D2 两台设备先后升级：两台旧版设备已配对并有历史；A 先升级并与仍为旧版的 B 互通，B 再升级。

use uc_testkit::ScenarioFailure;

use crate::{
    catalog::Point,
    cell::CellRun,
    device::{Device, write_representative_content},
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
