use std::{cell::RefCell, rc::Rc};

use js_sys::{Math, RegExp};
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use web_sys::{
    AudioBuffer, AudioContext, AudioContextState, AudioNode, AudioScheduledSourceNode,
    BiquadFilterNode, BiquadFilterType, Document, Element, Event, GainNode,
    OscillatorType, PageTransitionEvent, Window,
};

use crate::preferences::UiPreferences;

const SELECTOR: &str = "button,a[href],input:not([type='hidden']),select,textarea,summary,[role='button'],[role='checkbox'],[role='menuitem'],[role='menuitemcheckbox'],[role='menuitemradio'],[role='option'],[role='radio'],[role='switch'],[role='tab']";
const SILENCE: f64 = 0.0001;

thread_local! {
    static LISTENER: RefCell<Option<Listener>> = const { RefCell::new(None) };
    static AUDIO: RefCell<AudioState> = const { RefCell::new(AudioState { context: None, master: None, noise: None, down_at: f64::NEG_INFINITY, played_at: f64::NEG_INFINITY }) };
    static RELEASE_LABEL: RegExp = RegExp::new("close|dismiss|remove|delete|collapse|cancel|clear", "i");
    static PULSE_LABEL: RegExp = RegExp::new("send|save|submit|calculate|create|add|upgrade|replay", "i");
}

struct Listener {
    document: Document,
    window: Window,
    click: Closure<dyn FnMut(Event)>,
    pagehide: Closure<dyn FnMut(Event)>,
}

impl Drop for Listener {
    fn drop(&mut self) {
        let _ = self.document.remove_event_listener_with_callback_and_bool(
            "click",
            self.click.as_ref().unchecked_ref(),
            true,
        );
        let _ = self.window.remove_event_listener_with_callback(
            "pagehide",
            self.pagehide.as_ref().unchecked_ref(),
        );
    }
}

struct AudioState {
    context: Option<AudioContext>,
    master: Option<GainNode>,
    /// Half a second of white noise for this context: every noise layer plays a slice of it.
    noise: Option<AudioBuffer>,
    /// When a molded key last sounded its down-stroke, so its activation sounds the return.
    down_at: f64,
    /// When the last sound played, so a popup the same press opened stays silent.
    played_at: f64,
}

#[derive(Clone, Copy)]
pub(super) enum Cue {
    Press,
    Tick,
    Release,
    Page,
    Pulse,
    /// A pointer landing on a key; `true` for the primary key. Only a molded theme sounds it.
    KeyDown(bool),
    /// A latch going on (`true`) or off.
    Latch(bool),
    /// A key going down in a text field, or (`None`) coming back up. Only a molded theme types.
    Type(Option<Typed>),
    /// A popup opening without a press, such as a menu a typed trigger opens.
    Open,
}

/// The kind of key typed in a text field.
#[derive(Clone, Copy)]
pub(super) enum Typed {
    Key,
    Space,
    /// Enter, Backspace, Delete and Tab: bigger keys.
    Big,
    /// A held key repeating.
    Repeat,
}

/// The molded theme's voices, synthesized from noise bursts and falling tones.
#[derive(Clone, Copy)]
enum Voice {
    Down,
    DownPrimary,
    Up,
    LatchDown,
    LatchUp,
    Tick,
    Type,
    TypeSpace,
    TypeBig,
    TypeUp,
    Open,
}

/// The molded master level relative to the flat bus, which is set for the flat cues.
const VOICE_LEVEL: f64 = 0.55 / 0.32;
/// How long after its down-stroke a key's activation still sounds the return, in milliseconds.
const KEY_RETURN_MS: f64 = 1500.0;
/// A popup this soon after another sound opened from that press, which already sounded.
const OPEN_QUIET_MS: f64 = 150.0;

#[derive(Clone, Copy)]
enum Source {
    Noise,
    Sine(f64, Option<f64>),
    Square(f64),
}

#[derive(Clone, Copy)]
enum Filter {
    Highpass(f64),
    Bandpass(f64, f64),
    Lowpass(f64, f64),
}

#[derive(Clone, Copy)]
struct Layer {
    source: Source,
    filter: Option<Filter>,
    attack: f64,
    decay: f64,
    release: f64,
    gain: f64,
}

const PRESS: [Layer; 2] = [
    Layer {
        source: Source::Noise,
        filter: Some(Filter::Highpass(2400.0)),
        attack: 0.0003,
        decay: 0.009,
        release: 0.0,
        gain: 0.13,
    },
    Layer {
        source: Source::Sine(680.0, None),
        filter: None,
        attack: 0.0006,
        decay: 0.022,
        release: 0.008,
        gain: 0.24,
    },
];
const TICK: [Layer; 1] = [Layer {
    source: Source::Square(2100.0),
    filter: Some(Filter::Bandpass(2600.0, 1.6)),
    attack: 0.0004,
    decay: 0.028,
    release: 0.0,
    gain: 0.24,
}];
const RELEASE: [Layer; 1] = [Layer {
    source: Source::Noise,
    filter: Some(Filter::Lowpass(1600.0, 0.9)),
    attack: 0.001,
    decay: 0.055,
    release: 0.0,
    gain: 0.32,
}];
const PAGE: [Layer; 1] = [Layer {
    source: Source::Sine(430.0, Some(640.0)),
    filter: None,
    attack: 0.002,
    decay: 0.11,
    release: 0.03,
    gain: 0.4,
}];
const PULSE: [Layer; 1] = [Layer {
    source: Source::Sine(330.0, None),
    filter: Some(Filter::Lowpass(2200.0, 1.0)),
    attack: 0.002,
    decay: 0.13,
    release: 0.04,
    gain: 0.5,
}];

pub(super) fn install(document: &Document, preferences: Rc<UiPreferences>) -> Result<(), JsValue> {
    if LISTENER.with(|slot| slot.borrow().is_some()) {
        return Ok(());
    }
    let window = document
        .default_view()
        .ok_or_else(|| JsValue::from_str("document has no default window"))?;
    let click = Closure::wrap(Box::new(move |event: Event| {
        if preferences
            .snapshot()
            .is_ok_and(|snapshot| snapshot.sounds_enabled())
        {
            let _ = on_click(event);
        }
    }) as Box<dyn FnMut(Event)>);
    let mut cleanup_pending = false;
    let cleanup_window = window.clone();
    let pagehide = Closure::wrap(Box::new(move |event: Event| {
        if event
            .dyn_ref::<PageTransitionEvent>()
            .is_some_and(PageTransitionEvent::persisted)
        {
            return;
        }
        if cleanup_pending {
            return;
        }
        cleanup_pending = true;
        let cleanup = Closure::once_into_js(|| {
            LISTENER.with(|slot| drop(slot.borrow_mut().take()));
        });
        cleanup_window.queue_microtask(cleanup.unchecked_ref::<js_sys::Function>());
    }) as Box<dyn FnMut(Event)>);
    document.add_event_listener_with_callback_and_bool(
        "click",
        click.as_ref().unchecked_ref(),
        true,
    )?;
    if let Err(error) =
        window.add_event_listener_with_callback("pagehide", pagehide.as_ref().unchecked_ref())
    {
        let _ = document.remove_event_listener_with_callback_and_bool(
            "click",
            click.as_ref().unchecked_ref(),
            true,
        );
        return Err(error);
    }
    LISTENER.with(|slot| {
        *slot.borrow_mut() = Some(Listener {
            document: document.clone(),
            window,
            click,
            pagehide,
        })
    });
    Ok(())
}

fn on_click(event: Event) -> Result<(), JsValue> {
    let Some(target) = event
        .target()
        .and_then(|target| target.dyn_into::<Element>().ok())
    else {
        return Ok(());
    };
    #[cfg(feature = "rune-ui")]
    if target.closest("[data-rune-a11y]")?.is_some() {
        return Ok(());
    }
    let Some(control) = target.closest(SELECTOR)? else {
        return Ok(());
    };
    if control.closest("[data-sound-silent]")?.is_some()
        || control.matches(":disabled, [aria-disabled='true']")?
    {
        return Ok(());
    }
    let cue = cue_for(&control);
    AUDIO.with(|audio| play(audio, cue))
}

#[cfg(feature = "rune-ui")]
pub(super) fn rune(
    cue: Cue,
    snapshot: Option<crate::preferences::UiPreferenceSnapshot>,
) -> Result<(), JsValue> {
    let Some(snapshot) = snapshot.filter(|snapshot| snapshot.sounds_enabled()) else {
        return Ok(());
    };
    if snapshot.theme_key() == "hardware" {
        return AUDIO.with(|audio| play_molded(audio, cue));
    }
    let cue = match cue {
        Cue::Latch(_) => Cue::Tick,
        Cue::KeyDown(_) | Cue::Type(_) | Cue::Open => return Ok(()),
        cue => cue,
    };
    AUDIO.with(|audio| play(audio, cue))
}

fn cue_for(element: &Element) -> Cue {
    match element.get_attribute("data-sound").as_deref() {
        Some("press") => return Cue::Press,
        Some("tick") => return Cue::Tick,
        Some("release") => return Cue::Release,
        Some("page") => return Cue::Page,
        Some("pulse") => return Cue::Pulse,
        _ => {}
    }
    let label = format!(
        "{} {}",
        element.get_attribute("aria-label").unwrap_or_default(),
        element.text_content().unwrap_or_default()
    )
    .trim()
    .to_owned();
    if RELEASE_LABEL.with(|pattern| pattern.test(&label)) {
        return Cue::Release;
    }
    if element.matches("input[type='checkbox'], input[type='radio'], select, [role='checkbox'], [role='radio'], [role='switch'], [role='tab'], [aria-pressed]").unwrap_or(false) { return Cue::Tick; }
    if element.matches("a[href]").unwrap_or(false) {
        return Cue::Page;
    }
    if PULSE_LABEL.with(|pattern| pattern.test(&label)) {
        return Cue::Pulse;
    }
    if element.matches("input, textarea").unwrap_or(false) {
        return Cue::Tick;
    }
    Cue::Press
}

/// The context, the bus every sound plays into and this context's noise, made on first use.
fn ready(audio: &mut AudioState) -> Result<(AudioContext, AudioNode, AudioBuffer), JsValue> {
    let context = get_context(audio)?;
    let master = match &audio.master {
        Some(master) => master.clone(),
        None => {
            let master = context.create_gain()?;
            master.gain().set_value(0.32);
            master.connect_with_audio_node(&context.destination())?;
            audio.master = Some(master.clone());
            master
        }
    };
    let noise = match &audio.noise {
        Some(noise) => noise.clone(),
        None => {
            let noise = white_noise(&context, NOISE_SECONDS)?;
            audio.noise = Some(noise.clone());
            noise
        }
    };
    Ok((context, master.unchecked_into(), noise))
}

fn play(audio: &RefCell<AudioState>, cue: Cue) -> Result<(), JsValue> {
    let mut audio = audio.borrow_mut();
    let (context, destination, noise) = ready(&mut audio)?;
    let layers: &[Layer] = match cue {
        Cue::Press => &PRESS,
        Cue::Tick | Cue::Latch(_) => &TICK,
        Cue::Release => &RELEASE,
        Cue::Page => &PAGE,
        Cue::Pulse => &PULSE,
        Cue::KeyDown(_) | Cue::Type(_) | Cue::Open => return Ok(()),
    };
    let start = context.current_time();
    for layer in layers {
        play_layer(&context, &destination, &noise, *layer, start)?;
    }
    Ok(())
}

/// A molded theme's cue as a voice: a key sounds its down-stroke as the pointer lands and its
/// return on activation (or the down-stroke, from the keyboard); text fields type.
fn play_molded(audio: &RefCell<AudioState>, cue: Cue) -> Result<(), JsValue> {
    let mut audio = audio.borrow_mut();
    let now = js_sys::Date::now();
    let voice = match cue {
        Cue::KeyDown(primary) => {
            audio.down_at = now;
            if primary { Voice::DownPrimary } else { Voice::Down }
        }
        Cue::Press | Cue::Page | Cue::Release | Cue::Pulse if now - audio.down_at < KEY_RETURN_MS => {
            audio.down_at = f64::NEG_INFINITY;
            Voice::Up
        }
        Cue::Pulse => Voice::DownPrimary,
        Cue::Press | Cue::Page | Cue::Release => Voice::Down,
        Cue::Tick | Cue::Type(Some(Typed::Repeat)) => Voice::Tick,
        Cue::Latch(true) => Voice::LatchDown,
        Cue::Latch(false) => Voice::LatchUp,
        Cue::Type(Some(Typed::Key)) => Voice::Type,
        Cue::Type(Some(Typed::Space)) => Voice::TypeSpace,
        Cue::Type(Some(Typed::Big)) => Voice::TypeBig,
        Cue::Type(None) => Voice::TypeUp,
        Cue::Open if now - audio.played_at < OPEN_QUIET_MS => return Ok(()),
        Cue::Open => Voice::Open,
    };
    audio.played_at = now;
    let (context, destination, noise) = ready(&mut audio)?;
    let at = context.current_time() + 0.002;
    for part in voice.parts() {
        part.play(&context, &destination, &noise, at)?;
    }
    Ok(())
}

fn get_context(audio: &mut AudioState) -> Result<AudioContext, JsValue> {
    let context = match audio.context.as_ref() {
        Some(context) if context.state() != AudioContextState::Closed => context.clone(),
        _ => {
            let context = AudioContext::new()?;
            audio.master = None;
            audio.noise = None;
            context
        }
    };
    audio.context = Some(context.clone());
    if context.state() == AudioContextState::Suspended {
        let _ = context.resume()?;
    }
    Ok(context)
}

fn play_layer(
    context: &AudioContext,
    destination: &AudioNode,
    noise: &AudioBuffer,
    layer: Layer,
    start: f64,
) -> Result<(), JsValue> {
    let duration = layer.attack + layer.decay + layer.release;
    let end = start + duration;
    let envelope = context.create_gain()?;
    let gain = envelope.gain();
    gain.set_value_at_time(SILENCE as f32, start)?;
    if layer.attack > 0.0 {
        gain.linear_ramp_to_value_at_time(layer.gain as f32, start + layer.attack)?;
    } else {
        gain.set_value_at_time(layer.gain as f32, start)?;
    }
    gain.set_target_at_time(SILENCE as f32, start + layer.attack, layer.decay / 3.0)?;

    let (source, scheduled): (AudioNode, AudioNode) = match layer.source {
        Source::Noise => {
            let node = context.create_buffer_source()?;
            node.set_buffer(Some(noise));
            let slack = (NOISE_SECONDS - duration - 0.1).max(0.0);
            node.start_with_when_and_grain_offset(start, Math::random() * slack)?;
            node.unchecked_ref::<AudioScheduledSourceNode>()
                .stop_with_when(end + 0.1)?;
            (node.clone().unchecked_into(), node.unchecked_into())
        }
        Source::Sine(frequency, sweep) => {
            let node = context.create_oscillator()?;
            node.set_type(OscillatorType::Sine);
            node.frequency()
                .set_value_at_time(frequency as f32, start)?;
            if let Some(end_frequency) = sweep {
                node.frequency()
                    .exponential_ramp_to_value_at_time(end_frequency.max(1.0) as f32, end)?;
            }
            node.start_with_when(start)?;
            node.unchecked_ref::<AudioScheduledSourceNode>()
                .stop_with_when(end + 0.1)?;
            (node.clone().unchecked_into(), node.unchecked_into())
        }
        Source::Square(frequency) => {
            let node = context.create_oscillator()?;
            node.set_type(OscillatorType::Square);
            node.frequency()
                .set_value_at_time(frequency as f32, start)?;
            node.start_with_when(start)?;
            node.unchecked_ref::<AudioScheduledSourceNode>()
                .stop_with_when(end + 0.1)?;
            (node.clone().unchecked_into(), node.unchecked_into())
        }
    };
    let mut tail = source.clone();
    let filter = if let Some(filter) = layer.filter {
        let node = context.create_biquad_filter()?;
        match filter {
            Filter::Highpass(frequency) => {
                node.set_type(BiquadFilterType::Highpass);
                set_filter(&node, frequency, 1.0, start)?;
            }
            Filter::Bandpass(frequency, resonance) => {
                node.set_type(BiquadFilterType::Bandpass);
                set_filter(&node, frequency, resonance, start)?;
            }
            Filter::Lowpass(frequency, resonance) => {
                node.set_type(BiquadFilterType::Lowpass);
                set_filter(&node, frequency, resonance, start)?;
            }
        }
        tail.connect_with_audio_node(node.as_ref())?;
        tail = node.clone().unchecked_into();
        Some(node)
    } else {
        None
    };
    tail.connect_with_audio_node(envelope.as_ref())?;
    envelope.connect_with_audio_node(destination)?;

    let scheduled: AudioScheduledSourceNode = scheduled.unchecked_into();
    let scheduled_for_cleanup = scheduled.clone();
    let disconnect_source = source.clone();
    let disconnect_envelope: AudioNode = envelope.clone().unchecked_into();
    let disconnect_filter = filter.map(|node| node.unchecked_into::<AudioNode>());
    let ended = Closure::once_into_js(move || {
        scheduled_for_cleanup.set_onended(None);
        let _ = disconnect_source.disconnect();
        let _ = disconnect_envelope.disconnect();
        if let Some(filter) = disconnect_filter {
            let _ = filter.disconnect();
        }
    });
    if let Some(function) = ended.dyn_ref::<js_sys::Function>() {
        scheduled.set_onended(Some(function));
    }
    Ok(())
}

fn set_filter(
    node: &BiquadFilterNode,
    frequency: f64,
    resonance: f64,
    start: f64,
) -> Result<(), JsValue> {
    node.frequency()
        .set_value_at_time(frequency as f32, start)?;
    node.q().set_value(resonance as f32);
    Ok(())
}

/// Seconds of noise each context keeps; every noise layer plays a slice from a random offset.
const NOISE_SECONDS: f64 = 0.5;

fn white_noise(context: &AudioContext, duration: f64) -> Result<AudioBuffer, JsValue> {
    let length = (context.sample_rate() as f64 * duration) as u32;
    let buffer = context.create_buffer(1, length, context.sample_rate())?;
    // One seed from the browser, then xorshift in wasm: no host call per sample.
    let mut x = (Math::random() * u32::MAX as f64) as u32 | 1;
    let samples = (0..length)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            (x as f32 / u32::MAX as f32) * 2.0 - 1.0
        })
        .collect::<Vec<_>>();
    buffer.copy_to_channel(&samples, 0)?;
    Ok(buffer)
}

fn vary(amount: f64) -> f64 {
    1.0 + (Math::random() * 2.0 - 1.0) * amount
}

/// One part of a voice, `delay` seconds after it starts: a burst of filtered noise (plastic on
/// plastic) or a tone falling from `freq` to `to` (a key's body or a piezo).
#[derive(Clone, Copy)]
enum Part {
    Burst { delay: f64, highpass: bool, freq: f64, q: f64, level: f64, decay: f64 },
    Tone { delay: f64, square: bool, freq: f64, to: f64, level: f64, decay: f64, attack: f64 },
}

const fn burst(freq: f64, q: f64, level: f64, decay: f64) -> Part {
    Part::Burst { delay: 0.0, highpass: false, freq, q, level, decay }
}

const fn tone(freq: f64, to: f64, level: f64, decay: f64) -> Part {
    Part::Tone { delay: 0.0, square: false, freq, to, level, decay, attack: 0.001 }
}

const fn later(part: Part, by: f64) -> Part {
    match part {
        Part::Burst { highpass, freq, q, level, decay, .. } => Part::Burst { delay: by, highpass, freq, q, level, decay },
        Part::Tone { square, freq, to, level, decay, attack, .. } => Part::Tone { delay: by, square, freq, to, level, decay, attack },
    }
}

const DOWN: [Part; 2] = [burst(2600.0, 0.9, 0.32, 0.022), tone(210.0, 130.0, 0.4, 0.05)];
const DOWN_PRIMARY: [Part; 2] = [burst(2100.0, 0.8, 0.36, 0.026), tone(165.0, 100.0, 0.55, 0.07)];
const UP: [Part; 2] = [burst(3900.0, 1.2, 0.16, 0.012), tone(300.0, 240.0, 0.1, 0.018)];
const LATCH_DOWN: [Part; 3] = [
    burst(2000.0, 0.8, 0.18, 0.018),
    tone(160.0, 110.0, 0.2, 0.045),
    later(burst(1150.0, 14.0, 0.035, 0.05), 0.004),
];
const LATCH_UP: [Part; 2] = [burst(3200.0, 1.0, 0.11, 0.012), later(burst(1600.0, 12.0, 0.03, 0.04), 0.003)];
const TICK_VOICE: [Part; 2] = [
    Part::Burst { delay: 0.0, highpass: true, freq: 3200.0, q: 0.7, level: 0.2, decay: 0.006 },
    tone(4300.0, 4300.0, 0.035, 0.01),
];
const TYPE: [Part; 2] = [burst(3300.0, 1.1, 0.22, 0.014), tone(260.0, 190.0, 0.18, 0.03)];
const TYPE_SPACE: [Part; 3] = [
    burst(2200.0, 0.8, 0.26, 0.022),
    tone(170.0, 120.0, 0.26, 0.05),
    later(burst(900.0, 6.0, 0.05, 0.03), 0.006),
];
const TYPE_BIG: [Part; 2] = [burst(2600.0, 0.9, 0.26, 0.018), tone(200.0, 140.0, 0.28, 0.045)];
const TYPE_UP: [Part; 1] = [burst(4400.0, 1.3, 0.08, 0.008)];
const OPEN: [Part; 2] = [
    burst(2600.0, 1.0, 0.1, 0.012),
    Part::Tone { delay: 0.0, square: false, freq: 520.0, to: 820.0, level: 0.045, decay: 0.06, attack: 0.003 },
];

impl Voice {
    fn parts(self) -> &'static [Part] {
        match self {
            Voice::Down => &DOWN,
            Voice::DownPrimary => &DOWN_PRIMARY,
            Voice::Up => &UP,
            Voice::LatchDown => &LATCH_DOWN,
            Voice::LatchUp => &LATCH_UP,
            Voice::Tick => &TICK_VOICE,
            Voice::Type => &TYPE,
            Voice::TypeSpace => &TYPE_SPACE,
            Voice::TypeBig => &TYPE_BIG,
            Voice::TypeUp => &TYPE_UP,
            Voice::Open => &OPEN,
        }
    }
}

impl Part {
    /// Plays the part at `at`, each time a little different in pitch and level, as no two
    /// presses of a real switch sound alike; its nodes disconnect when it ends.
    fn play(self, context: &AudioContext, destination: &AudioNode, noise: &AudioBuffer, at: f64) -> Result<(), JsValue> {
        let (delay, level, decay, attack) = match self {
            Part::Burst { delay, level, decay, .. } => (delay, level * vary(0.15), decay, 0.0008),
            Part::Tone { delay, level, decay, attack, .. } => (delay, level * vary(0.12), decay, attack),
        };
        let at = at + delay;
        let envelope = context.create_gain()?;
        let gain = envelope.gain();
        gain.set_value_at_time(0.0, at)?;
        gain.linear_ramp_to_value_at_time((level * VOICE_LEVEL) as f32, at + attack)?;
        gain.exponential_ramp_to_value_at_time(SILENCE as f32, at + decay)?;
        envelope.connect_with_audio_node(destination)?;
        let (source, filter): (AudioScheduledSourceNode, Option<AudioNode>) = match self {
            Part::Burst { highpass, freq, q, .. } => {
                let node = context.create_buffer_source()?;
                node.set_buffer(Some(noise));
                node.playback_rate().set_value(vary(0.1) as f32);
                let filter = context.create_biquad_filter()?;
                filter.set_type(if highpass { BiquadFilterType::Highpass } else { BiquadFilterType::Bandpass });
                filter.frequency().set_value((freq * vary(0.06)) as f32);
                filter.q().set_value(q as f32);
                node.connect_with_audio_node(&filter)?;
                filter.connect_with_audio_node(&envelope)?;
                node.start_with_when_and_grain_offset(at, Math::random() * 0.4)?;
                (node.unchecked_into(), Some(filter.unchecked_into()))
            }
            Part::Tone { square, freq, to, .. } => {
                let node = context.create_oscillator()?;
                node.set_type(if square { OscillatorType::Square } else { OscillatorType::Sine });
                let f = freq * vary(0.04);
                node.frequency().set_value_at_time(f as f32, at)?;
                node.frequency().exponential_ramp_to_value_at_time((to / freq * f).max(20.0) as f32, at + decay)?;
                node.connect_with_audio_node(&envelope)?;
                node.start_with_when(at)?;
                (node.unchecked_into(), None)
            }
        };
        source.stop_with_when(at + decay + 0.02)?;
        let ended_source: AudioNode = source.clone().unchecked_into();
        let ended_envelope: AudioNode = envelope.unchecked_into();
        let cleanup = source.clone();
        let ended = Closure::once_into_js(move || {
            cleanup.set_onended(None);
            let _ = ended_source.disconnect();
            let _ = ended_envelope.disconnect();
            if let Some(filter) = filter {
                let _ = filter.disconnect();
            }
        });
        source.set_onended(ended.dyn_ref::<js_sys::Function>());
        Ok(())
    }
}
