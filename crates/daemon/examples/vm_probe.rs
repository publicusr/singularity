//! The Soloist VM on its own (macOS): `vm_probe <data dir>`. Prepares and boots it, has the guest fetch Soloist, then
//! stops it. A scratch data dir keeps it apart from the real one.
#[cfg(target_os = "macos")]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let dir = std::path::PathBuf::from(std::env::args().nth(1).ok_or_else(|| anyhow::anyhow!("usage: vm_probe <data dir>"))?);
    let vm = singularityd::vm::Vm::new(&dir);
    let t = std::time::Instant::now();
    let say = |s: String| println!("[{:5.1}s] {s}", t.elapsed().as_secs_f32());
    vm.prepare(&say).await?;
    say("prepared".into());
    vm.start(&say).await?;
    say("up".into());
    say(format!("guest: {}", vm.ssh("uname -m; free -m | sed -n 2p; systemctl --user is-active pulseaudio sgl-pcm").await?.replace('\n', " | ")));
    say(format!("soloist: {}", vm.ssh("sgl-soloist ensure").await?.trim()));
    if std::env::args().any(|a| a == "--trim") {
        say(format!("trim: {}", vm.ssh("sudo fstrim -av; df -h / | tail -1").await?.replace('\n', " | ")));
    }
    vm.stop().await;
    say("stopped".into());
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn main() {}
