//! Any bytes on the socket, read as a Client's and as a Daemon's messages: no panic, no runaway allocation.
#![no_main]
use libfuzzer_sys::fuzz_target;
use singularity_proto::{ClientMsg, DaemonMsg, read_msg};

fuzz_target!(|data: &[u8]| {
    let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
    rt.block_on(async {
        let mut rd = data;
        while let Ok(Some(_)) = read_msg::<_, ClientMsg>(&mut rd).await {}
        let mut rd = data;
        while let Ok(Some(_)) = read_msg::<_, DaemonMsg>(&mut rd).await {}
    });
});
