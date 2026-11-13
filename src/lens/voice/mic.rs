//! The microphone at 16 kHz mono, through `AVAudioEngine`. Ported from attn
//! 94d1c6551 `app/src-tauri/core/src/dictation.rs`.
use super::{Feed, SAMPLE_RATE, Source};
use crate::{
    permission_guide::{self, Permission},
    platform_port::{Sink, mac::host_time},
};
use block2::RcBlock;
use objc2::{AllocAnyThread, rc::Retained, runtime::Bool};
use objc2_av_foundation::{AVAuthorizationStatus, AVCaptureDevice, AVMediaTypeAudio};
use objc2_avf_audio::{
    AVAudioBuffer, AVAudioCommonFormat, AVAudioConverter, AVAudioConverterInputStatus,
    AVAudioEngine, AVAudioEngineConfigurationChangeNotification, AVAudioFormat, AVAudioPCMBuffer,
    AVAudioTime,
};
use objc2_foundation::{NSNotification, NSNotificationCenter, NSObjectProtocol};
use std::{
    cell::{Cell, RefCell},
    ptr::NonNull,
};

/// Frames per tap buffer at the hardware rate; AVAudioEngine may deliver other sizes.
const TAP_FRAMES: u32 = 1_600;

const DENIED: &str = "Attn Lens does not have microphone access. Allow it in System Settings › Privacy & Security › Microphone, then press right ⌥ again.";

struct Microphone {
    engine: Retained<AVAudioEngine>,
    observer: Retained<objc2::runtime::ProtocolObject<dyn NSObjectProtocol>>,
}

// SAFETY: AVAudioEngine and the observer token are used from one thread at a
// time: the voice thread opens them, and `stop` runs once, after that.
unsafe impl Send for Microphone {}

impl Source for Microphone {
    fn stop(self: Box<Self>) {
        // SAFETY: stopping an engine this module started, and removing its observer.
        unsafe {
            self.engine.inputNode().removeTapOnBus(0);
            self.engine.stop();
            NSNotificationCenter::defaultCenter().removeObserver(self.observer.as_ref());
        }
    }
}

/// Asks for the microphone the first time; afterwards the guide shows the way to
/// System Settings.
fn access() -> Result<(), String> {
    // SAFETY: a constant AVFoundation exports.
    let media = unsafe { AVMediaTypeAudio }.ok_or("AVFoundation has no audio media type")?;
    // SAFETY: a class query that needs no permission.
    let status = unsafe { AVCaptureDevice::authorizationStatusForMediaType(media) };
    if status == AVAuthorizationStatus::Authorized {
        return Ok(());
    }
    if status == AVAuthorizationStatus::NotDetermined {
        let (answer, granted) = std::sync::mpsc::channel();
        let handler = RcBlock::new(move |allowed: Bool| {
            let _ = answer.send(allowed.as_bool());
        });
        // SAFETY: the handler matches the method's block type and outlives the call.
        unsafe { AVCaptureDevice::requestAccessForMediaType_completionHandler(media, &handler) };
        if granted.recv() == Ok(true) {
            return Ok(());
        }
    }
    permission_guide::show(Permission::Microphone);
    Err(DENIED.into())
}

pub fn open(feed: Feed, failed: Sink<String>) -> Result<Box<dyn Source>, String> {
    access()?;
    // SAFETY: each call below is AVAudioEngine's documented setup on an engine
    // and formats this function owns; the tap and observer blocks match the
    // signatures AVFoundation expects.
    unsafe {
        let engine = AVAudioEngine::new();
        let input = engine.inputNode();
        let hardware = input.outputFormatForBus(0);
        let rate = hardware.sampleRate();
        if rate <= 0.0 || hardware.channelCount() == 0 {
            return Err("No microphone is available. Connect one and press right ⌥ again.".into());
        }
        let speech = AVAudioFormat::initWithCommonFormat_sampleRate_channels_interleaved(
            AVAudioFormat::alloc(),
            AVAudioCommonFormat::PCMFormatFloat32,
            f64::from(SAMPLE_RATE),
            1,
            false,
        )
        .ok_or("Cannot describe 16 kHz mono audio")?;
        let converter = AVAudioConverter::initFromFormat_toFormat(
            AVAudioConverter::alloc(),
            &hardware,
            &speech,
        )
        .ok_or_else(|| format!("Cannot convert {rate} Hz microphone audio to 16 kHz"))?;
        converter.setDownmix(true);
        // AVAudioEngine calls the tap on one thread at a time.
        let feed = RefCell::new(feed);
        let first = Cell::new(true);
        let tap_failed = failed.clone();
        let tap = RcBlock::new(
            move |buffer: NonNull<AVAudioPCMBuffer>, when: NonNull<AVAudioTime>| {
                let samples = match convert(&converter, &speech, buffer.as_ref(), rate) {
                    Ok(samples) => samples,
                    Err(error) => return tap_failed(error),
                };
                // The host time of the first buffer's first sample puts phrases on the capture clock.
                let start = if first.replace(false) {
                    let when = when.as_ref();
                    if !when.isHostTimeValid() {
                        return tap_failed("The microphone gave audio with no time.".into());
                    }
                    Some(host_time(when.hostTime()))
                } else {
                    None
                };
                (feed.borrow_mut())(&samples, start);
            },
        );
        input.installTapOnBus_bufferSize_format_block(
            0,
            TAP_FRAMES,
            Some(&hardware),
            RcBlock::as_ptr(&tap),
        );
        let changed = RcBlock::new(move |_: NonNull<NSNotification>| {
            failed("The microphone changed. Press right ⌥ to start voice again.".into());
        });
        let observer = NSNotificationCenter::defaultCenter()
            .addObserverForName_object_queue_usingBlock(
                Some(AVAudioEngineConfigurationChangeNotification),
                Some(&engine),
                None,
                &changed,
            );
        engine.prepare();
        if let Err(error) = engine.startAndReturnError() {
            input.removeTapOnBus(0);
            NSNotificationCenter::defaultCenter().removeObserver(observer.as_ref());
            return Err(format!(
                "The microphone did not start: {}",
                error.localizedDescription()
            ));
        }
        Ok(Box::new(Microphone { engine, observer }))
    }
}

fn convert(
    converter: &AVAudioConverter,
    format: &AVAudioFormat,
    buffer: &AVAudioPCMBuffer,
    rate: f64,
) -> Result<Vec<f32>, String> {
    // SAFETY: the converter reads `buffer` once, inside this call, and writes
    // into `output`, which this function owns.
    unsafe {
        let frames = buffer.frameLength();
        let capacity = (f64::from(frames) * f64::from(SAMPLE_RATE) / rate).ceil() as u32 + 64;
        let output = AVAudioPCMBuffer::initWithPCMFormat_frameCapacity(
            AVAudioPCMBuffer::alloc(),
            format,
            capacity,
        )
        .ok_or("Cannot allocate a 16 kHz audio buffer")?;
        let supplied = Cell::new(false);
        let source: *const AVAudioPCMBuffer = buffer;
        let input = RcBlock::new(
            move |_: u32, status: NonNull<AVAudioConverterInputStatus>| -> *mut AVAudioBuffer {
                if supplied.replace(true) {
                    status.write(AVAudioConverterInputStatus::NoDataNow);
                    return std::ptr::null_mut();
                }
                status.write(AVAudioConverterInputStatus::HaveData);
                source.cast_mut().cast()
            },
        );
        let mut error = None;
        converter.convertToBuffer_error_withInputFromBlock(
            &output,
            Some(&mut error),
            RcBlock::as_ptr(&input),
        );
        if let Some(error) = error {
            return Err(format!(
                "Microphone audio could not be converted: {}",
                error.localizedDescription()
            ));
        }
        let length = output.frameLength() as usize;
        let channels = output.floatChannelData();
        if channels.is_null() || length == 0 {
            return Ok(Vec::new());
        }
        Ok(std::slice::from_raw_parts((*channels).as_ptr(), length).to_vec())
    }
}
