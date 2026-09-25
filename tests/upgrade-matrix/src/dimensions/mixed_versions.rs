//! D3 新旧设备混用：一台旧版、一台新版从零开始配对并互通；邀请方分别为旧版与新版。

use uc_testkit::ScenarioFailure;

use crate::{
    catalog::{Inviter, Point},
    cell::CellRun,
    device::{Device, write_representative_content},
    fixture::representative,
    interop::{exchange, pair as pair_devices, remove_and_rejoin},
};

pub(crate) async fn pair(
    run: &mut CellRun,
    old: &Point,
    new: &Point,
    inviter: Inviter,
) -> Result<(), ScenarioFailure> {
    let (inviter_point, joiner_point) = match inviter {
        Inviter::Old => (old, new),
        Inviter::New => (new, old),
    };
    let mut a = Device::new(run, "a", "Device A")?;
    let mut b = Device::new(run, "b", "Device B")?;
    {
        let _stage = run.stage("pair");
        a.start(run, inviter_point).await?;
        b.start(run, joiner_point).await?;
        pair_devices(&mut a, &mut b, true).await?;
    }
    {
        let _stage = run.stage("interop");
        write_representative_content(&mut a, &representative("a-0")).await?;
        exchange(run, &mut a, &mut b, "mixed").await?;
    }
    {
        let _stage = run.stage("continue");
        remove_and_rejoin(run, &mut a, &mut b, "rejoined").await?;
    }
    let _stage = run.stage("cleanup");
    a.stop(run).await?;
    b.stop(run).await
}
