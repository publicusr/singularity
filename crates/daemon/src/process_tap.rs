//! The AudioTap's source on macOS for an Engine that is another app (#67): a Core Audio process tap (macOS 14.2+) on
//! that app's processes, so the audio Widgets see exactly what it plays and nothing else.
//!
//! - A `CATapDescription` (stereo mixdown of the app's process objects, private, unmuted), made into a tap and wrapped
//!   in a private aggregate device whose IO callback receives the tapped audio (Apple's AudioCap sample does the same).
//! - Needs the "record audio from other apps" permission, asked for once; macOS gives it to the app that started the
//!   Daemon (the terminal). Without it the buffers are silent: [`ProcessTap::heard`] tells the two apart over time.
//! - The callback runs on Core Audio's real-time thread: it only converts and hands the samples on.

use objc2::AllocAnyThread;
use objc2_core_audio::*;
use objc2_core_audio_types::{AudioBufferList, AudioStreamBasicDescription, AudioTimeStamp};
use objc2_foundation::{NSArray, NSDictionary, NSNumber, NSString};
use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Interleaved stereo float samples and their sample rate, from the real-time thread.
pub type Sink = Box<dyn FnMut(&[f32], f64) + Send>;

struct Ctx {
    sink: Sink,
    rate: f64,
    heard: Arc<AtomicBool>,
}

/// A running tap; stopped and torn down when dropped.
pub struct ProcessTap {
    tap: AudioObjectID,
    aggregate: AudioObjectID,
    proc_id: AudioDeviceIOProcID,
    _ctx: Box<Ctx>,
    heard: Arc<AtomicBool>,
}

// (the ids are plain numbers; the context is only touched by Core Audio's thread until drop)
unsafe impl Send for ProcessTap {}

impl ProcessTap {
    /// Has anything but silence come through? (Silence for long while the app plays: no permission.)
    pub fn heard(&self) -> bool {
        self.heard.load(Ordering::Relaxed)
    }

    /// Tap the processes with these process objects (see [`objects_of`]).
    pub fn start(objects: &[AudioObjectID], sink: Sink) -> anyhow::Result<ProcessTap> {
        anyhow::ensure!(!objects.is_empty(), "no audio processes to tap");
        // SAFETY: Core Audio calls with valid pointers to locals or boxed data that outlive them; every id made here is
        // destroyed on the error paths and in Drop
        unsafe {
            let ids: Vec<objc2::rc::Retained<NSNumber>> = objects.iter().map(|o| NSNumber::new_u32(*o)).collect();
            let list = NSArray::from_retained_slice(&ids);
            let desc = CATapDescription::initStereoMixdownOfProcesses(CATapDescription::alloc(), &list);
            desc.setPrivate(true);
            desc.setMuteBehavior(CATapMuteBehavior::Unmuted);
            desc.setName(&NSString::from_str("Singularity"));
            let mut tap: AudioObjectID = 0;
            check(AudioHardwareCreateProcessTap(Some(&desc), &mut tap), "create the tap")?;
            let uuid = desc.UUID().UUIDString();
            let format: AudioStreamBasicDescription = match get(tap, kAudioTapPropertyFormat) {
                Some(f) => f,
                None => {
                    AudioHardwareDestroyProcessTap(tap);
                    anyhow::bail!("couldn't read the tap's format");
                }
            };
            // a private aggregate device holding only the tap
            let agg_uid = NSString::from_str(&format!("singularity-tap-{}", std::process::id()));
            let tap_entry = dict(&[("uid", obj(uuid)), ("drift", obj(NSNumber::new_bool(true)))]);
            let agg = dict(&[
                ("name", obj(NSString::from_str("Singularity tap"))),
                ("uid", obj(agg_uid)),
                ("private", obj(NSNumber::new_bool(true))),
                ("stacked", obj(NSNumber::new_bool(false))),
                ("tapautostart", obj(NSNumber::new_bool(true))),
                ("taps", obj(NSArray::from_retained_slice(&[tap_entry]))),
            ]);
            // (toll-free bridged)
            let cf: &objc2_core_foundation::CFDictionary = &*(objc2::rc::Retained::as_ptr(&agg) as *const objc2_core_foundation::CFDictionary);
            let mut aggregate: AudioObjectID = 0;
            if let Err(e) = check(AudioHardwareCreateAggregateDevice(cf, NonNull::from(&mut aggregate)), "create the aggregate device") {
                AudioHardwareDestroyProcessTap(tap);
                return Err(e);
            }
            let heard = Arc::new(AtomicBool::new(false));
            let mut ctx = Box::new(Ctx { sink, rate: format.mSampleRate, heard: heard.clone() });
            let mut proc_id: AudioDeviceIOProcID = None;
            let started = check(AudioDeviceCreateIOProcID(aggregate, Some(io_proc), (&mut *ctx as *mut Ctx).cast(), NonNull::from(&mut proc_id)), "add the IO callback")
                .and_then(|_| check(AudioDeviceStart(aggregate, proc_id), "start the aggregate device"));
            if let Err(e) = started {
                if proc_id.is_some() {
                    AudioDeviceDestroyIOProcID(aggregate, proc_id);
                }
                AudioHardwareDestroyAggregateDevice(aggregate);
                AudioHardwareDestroyProcessTap(tap);
                return Err(e);
            }
            Ok(ProcessTap { tap, aggregate, proc_id, _ctx: ctx, heard })
        }
    }
}

impl Drop for ProcessTap {
    fn drop(&mut self) {
        // SAFETY: the ids were made in `start` and are destroyed once, the callback stopped first
        unsafe {
            AudioDeviceStop(self.aggregate, self.proc_id);
            AudioDeviceDestroyIOProcID(self.aggregate, self.proc_id);
            AudioHardwareDestroyAggregateDevice(self.aggregate);
            AudioHardwareDestroyProcessTap(self.tap);
        }
    }
}

unsafe extern "C-unwind" fn io_proc(_: AudioObjectID, _: NonNull<AudioTimeStamp>, input: NonNull<AudioBufferList>, _: NonNull<AudioTimeStamp>, _: NonNull<AudioBufferList>, _: NonNull<AudioTimeStamp>, data: *mut c_void) -> i32 {
    // SAFETY: `data` is the boxed Ctx registered with this callback; Core Audio's buffers are valid for the call
    unsafe {
        let ctx = &mut *(data as *mut Ctx);
        let list = input.as_ref();
        if list.mNumberBuffers == 0 {
            return 0;
        }
        let b = &list.mBuffers[0];
        if b.mData.is_null() {
            return 0;
        }
        let samples = std::slice::from_raw_parts(b.mData as *const f32, b.mDataByteSize as usize / 4);
        if !ctx.heard.load(Ordering::Relaxed) && samples.iter().any(|s| s.abs() > 1e-4) {
            ctx.heard.store(true, Ordering::Relaxed);
        }
        (ctx.sink)(samples, ctx.rate);
    }
    0
}

/// Any Objective-C object, as a dictionary value.
fn obj<T: objc2::Message>(r: objc2::rc::Retained<T>) -> objc2::rc::Retained<objc2::runtime::AnyObject> {
    // SAFETY: every Objective-C object is an AnyObject
    unsafe { objc2::rc::Retained::cast_unchecked(r) }
}

fn dict(entries: &[(&str, objc2::rc::Retained<objc2::runtime::AnyObject>)]) -> objc2::rc::Retained<NSDictionary<NSString, objc2::runtime::AnyObject>> {
    let keys: Vec<objc2::rc::Retained<NSString>> = entries.iter().map(|(k, _)| NSString::from_str(k)).collect();
    let key_refs: Vec<&NSString> = keys.iter().map(|k| &**k).collect();
    let values: Vec<_> = entries.iter().map(|(_, v)| v.clone()).collect();
    NSDictionary::from_retained_objects(&key_refs, &values)
}

fn check(status: i32, what: &str) -> anyhow::Result<()> {
    if status == 0 { Ok(()) } else { anyhow::bail!("couldn't {what} (Core Audio error {status})") }
}

/// A fixed-size property of an audio object.
fn get<T: Copy>(object: AudioObjectID, selector: AudioObjectPropertySelector) -> Option<T> {
    let addr = AudioObjectPropertyAddress { mSelector: selector, mScope: kAudioObjectPropertyScopeGlobal, mElement: kAudioObjectPropertyElementMain };
    let mut value = std::mem::MaybeUninit::<T>::uninit();
    let mut size = std::mem::size_of::<T>() as u32;
    // SAFETY: the buffer is as large as said
    let st = unsafe { AudioObjectGetPropertyData(object, NonNull::from(&addr), 0, std::ptr::null(), NonNull::from(&mut size), NonNull::new_unchecked(value.as_mut_ptr().cast())) };
    // SAFETY: Core Audio filled it (status 0, full size)
    (st == 0 && size as usize == std::mem::size_of::<T>()).then(|| unsafe { value.assume_init() })
}

/// The audio process objects of the processes with these pids that are known to Core Audio.
pub fn objects_of(pids: &[i32]) -> Vec<AudioObjectID> {
    let addr = AudioObjectPropertyAddress { mSelector: kAudioHardwarePropertyProcessObjectList, mScope: kAudioObjectPropertyScopeGlobal, mElement: kAudioObjectPropertyElementMain };
    let mut size = 0u32;
    // SAFETY: size query, then a buffer of that size
    unsafe {
        if AudioObjectGetPropertyDataSize(kAudioObjectSystemObject as AudioObjectID, NonNull::from(&addr), 0, std::ptr::null(), NonNull::from(&mut size)) != 0 || size == 0 {
            return vec![];
        }
        let mut all = vec![0 as AudioObjectID; size as usize / 4];
        if AudioObjectGetPropertyData(kAudioObjectSystemObject as AudioObjectID, NonNull::from(&addr), 0, std::ptr::null(), NonNull::from(&mut size), NonNull::new_unchecked(all.as_mut_ptr().cast())) != 0 {
            return vec![];
        }
        all.truncate(size as usize / 4);
        all.into_iter().filter(|o| get::<i32>(*o, kAudioProcessPropertyPID).is_some_and(|p| pids.contains(&p))).collect()
    }
}

/// The Spotify app's processes ("Spotify", "Spotify Helper (…)"): which of them plays isn't fixed, so all are tapped.
pub fn spotify_pids() -> Vec<i32> {
    // SAFETY: libproc fills buffers of the sizes passed; counts are bounds-checked
    unsafe {
        let n = libc::proc_listallpids(std::ptr::null_mut(), 0);
        if n <= 0 {
            return vec![];
        }
        let mut pids = vec![0i32; n as usize + 64];
        let n = libc::proc_listallpids(pids.as_mut_ptr().cast(), (pids.len() * 4) as i32);
        pids.truncate(n.max(0) as usize);
        let mut out: Vec<i32> = pids
            .into_iter()
            .filter(|p| *p > 0)
            .filter(|p| {
                let mut buf = [0u8; 64];
                let len = libc::proc_name(*p, buf.as_mut_ptr().cast(), buf.len() as u32);
                let name = String::from_utf8_lossy(&buf[..len.max(0) as usize]).into_owned();
                name == "Spotify" || name.starts_with("Spotify Helper")
            })
            .collect();
        out.sort_unstable();
        out
    }
}

/// While the AudioTap wants samples and the Spotify app runs, keep a tap on it feeding `feed` (resampled to
/// 44.1 kHz); follow the app if it restarts. If macOS hands over only silence while the app plays, say once where the
/// permission is.
pub async fn manage(hub: Arc<crate::Hub>, feed: Arc<crate::tap::Feed>) {
    use std::time::{Duration, Instant};
    let mut tap: Option<(ProcessTap, Vec<i32>, Instant)> = None;
    let mut warned = false;
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        if !feed.wanted() {
            tap = None;
            continue;
        }
        let pids = spotify_pids();
        if pids.is_empty() {
            tap = None;
            continue;
        }
        if tap.as_ref().map(|t| &t.1) != Some(&pids) {
            let objects = objects_of(&pids);
            if objects.is_empty() {
                // the app runs but hasn't opened audio yet
                tap = None;
                continue;
            }
            let f = feed.clone();
            let mut resampler = crate::tap::Resampler::default();
            let mut out = Vec::with_capacity(4096);
            let sink: Sink = Box::new(move |samples, rate| {
                out.clear();
                resampler.push(samples, rate, &mut out);
                if f.wanted() {
                    f.push(out.clone());
                }
            });
            tap = match ProcessTap::start(&objects, sink) {
                Ok(t) => {
                    tracing::info!("audio tap on the Spotify app ({} processes)", pids.len());
                    Some((t, pids, Instant::now()))
                }
                Err(e) => {
                    tracing::warn!("audio tap on the Spotify app: {e}");
                    None
                }
            };
        }
        let playing = hub.playback.lock().await.status == singularity_proto::PlayStatus::Playing;
        if let Some((t, _, since)) = &tap
            && playing
            && !t.heard()
            && since.elapsed() > Duration::from_secs(6)
            && !warned
        {
            warned = true;
            let message = format!("{NO_PERMISSION} (then restart Singularity)");
            tracing::warn!("{message}");
            let _ = hub.events.send(singularity_proto::Event::Error { message });
        }
    }
}

/// What to do when the tap gets only silence: macOS gives the permission to the app Singularity runs in.
pub const NO_PERMISSION: &str = "The audio Widgets can't hear the Spotify app: macOS needs your terminal app allowed in System Settings → Privacy & Security → Screen & System Audio Recording → System Audio Recording Only";
