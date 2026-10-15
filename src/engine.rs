use crate::config::{Config, MischiefLevel};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mood {
    Idle,
    Happy,
    Grumpy,
    Sleep,
    Dragged,
    Play,
    Chase,
    Hiding,
    Dizzy,
    Walk,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interaction {
    Pet,
    Poke,
    Feed,
    Play,
    Drag,
    Release,
    Busy,
    Wake,
    StopMischief,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    HideIcons,
    RestoreIcons,
}

pub struct Engine {
    pub config: Config,
    pub mood: Mood,
    pub bubble: Option<&'static str>,
    pub mood_until: f64,
    pub last_interaction: f64,
    pub quiet_until: f64,
    pub bubble_until: f64,
    pub effect_until: f64,
    pub hidden: bool,
    pub stage: u8,
    pub next_antic: f64,
    pub history: Vec<String>,
    pub dirty: bool,
    random: u64,
    pokes: u8,
}

impl Engine {
    pub fn new(config: Config, now: f64, seed: u64) -> Self {
        Self {
            config,
            mood: Mood::Idle,
            bubble: Some("你好呀，我是你的小麻烦。"),
            mood_until: now + 4.0,
            last_interaction: now,
            quiet_until: 0.0,
            bubble_until: now + 5.0,
            effect_until: 0.0,
            hidden: false,
            stage: 0,
            next_antic: now + 25.0,
            history: vec!["搬进你的桌面。".into()],
            dirty: false,
            random: seed.max(1),
            pokes: 0,
        }
    }

    fn random(&mut self) -> u64 {
        self.random ^= self.random << 13;
        self.random ^= self.random >> 7;
        self.random ^= self.random << 17;
        self.random
    }

    pub fn say(&mut self, text: &'static str, mood: Mood, now: f64, seconds: f64) {
        self.mood = mood;
        self.mood_until = now + seconds;
        self.bubble = Some(text);
        self.bubble_until = now + seconds;
        self.record(text);
    }

    fn record(&mut self, text: &str) {
        if self.history.last().map(String::as_str) != Some(text) {
            self.history.push(text.to_owned());
            if self.history.len() > 30 {
                self.history.remove(0);
            }
        }
    }

    pub fn interact(&mut self, action: Interaction, now: f64) -> Option<Effect> {
        self.last_interaction = now;
        self.stage = 0;
        self.next_antic = now + 35.0;
        self.config.interactions = self.config.interactions.saturating_add(1);
        self.dirty = true;
        if action != Interaction::Poke {
            self.pokes = 0;
        }
        match action {
            Interaction::Pet => {
                self.config.affinity = (self.config.affinity + 2).min(100);
                self.say("好吧，再摸一下就原谅你。", Mood::Happy, now, 4.0);
            }
            Interaction::Poke => {
                self.pokes = self.pokes.saturating_add(1);
                if self.pokes >= 3 {
                    self.say("再戳我就戴头盔了！", Mood::Grumpy, now, 5.0);
                } else {
                    self.say("喂！我的脸不是按钮！", Mood::Dizzy, now, 3.0);
                }
            }
            Interaction::Feed => {
                self.config.feeds = self.config.feeds.saturating_add(1);
                self.config.affinity = (self.config.affinity + 3).min(100);
                self.say("嘴边这个？是皮肤纹理。", Mood::Happy, now, 5.0);
            }
            Interaction::Play => self.say("接招！这个球是我的了。", Mood::Play, now, 12.0),
            Interaction::Drag => self.say("放我下来！我不要面子的吗！", Mood::Dragged, now, 60.0),
            Interaction::Release => self.say("刚才摔倒的是我的替身。", Mood::Dizzy, now, 3.0),
            Interaction::Busy => {
                self.quiet_until = now + 30.0 * 60.0;
                self.say("你忙你的，我躺我的。", Mood::Sleep, now, 4.0);
            }
            Interaction::Wake => {
                self.quiet_until = 0.0;
                self.say("我没有睡！我在闭眼思考。", Mood::Happy, now, 4.0);
            }
            Interaction::StopMischief => {
                self.effect_until = now;
                self.say("好啦，不闹了，听你的。", Mood::Happy, now, 4.0);
            }
        }
        if self.hidden {
            self.hidden = false;
            Some(Effect::RestoreIcons)
        } else {
            None
        }
    }

    pub fn quiet(&self, now: f64) -> bool {
        self.config.quiet || now < self.quiet_until
    }

    pub fn mischief_active(&self) -> bool {
        self.hidden || self.mood == Mood::Chase
    }

    /// Stops transient behaviors for exit; the caller must also restore desktop effects.
    pub fn stop_mischief(&mut self, now: f64) {
        self.hidden = false;
        self.mood = Mood::Sleep;
        self.bubble = None;
        self.mood_until = now;
        self.bubble_until = now;
        self.effect_until = now;
        self.stage = 0;
        self.quiet_until = f64::INFINITY;
        self.next_antic = f64::INFINITY;
    }

    pub fn tick(&mut self, now: f64, desktop_active: bool) -> Option<Effect> {
        if self.hidden
            && (now >= self.effect_until
                || self.quiet(now)
                || !self.config.allow_hide_icons
                || self.config.level != MischiefLevel::Devil)
        {
            self.hidden = false;
            self.say("好啦好啦，全都还给你。", Mood::Happy, now, 4.0);
            return Some(Effect::RestoreIcons);
        }
        if now >= self.bubble_until {
            self.bubble = None;
        }
        if self.mood == Mood::Chase
            && (!self.config.allow_cursor_chase || self.config.level == MischiefLevel::Calm)
        {
            self.mood_until = now;
            self.mood = Mood::Idle;
        }
        if self.quiet(now) {
            self.mood = Mood::Sleep;
            self.bubble = None;
            return None;
        }
        if now < self.mood_until || self.hidden {
            return None;
        }
        self.mood = Mood::Idle;
        let neglected = now - self.last_interaction;
        let interval = self.config.neglect_seconds as f64;
        if self.config.level != MischiefLevel::Calm {
            if neglected >= interval * 3.0 && self.stage < 3 {
                self.stage = 3;
                if desktop_active {
                    if self.config.level == MischiefLevel::Devil
                        && self.config.allow_hide_icons
                        && self.config.hide_icons_consent_version == 1
                    {
                        self.begin_hide(now);
                        return Some(Effect::HideIcons);
                    }
                } else if self.config.allow_cursor_chase {
                    self.begin_chase(now);
                }
                return None;
            }
            if neglected >= interval * 2.0 && self.stage < 2 {
                self.stage = 2;
                self.say("我都躺好了，你还不摸摸？", Mood::Grumpy, now, 5.0);
                return None;
            }
            if neglected >= interval && self.stage < 1 {
                self.stage = 1;
                self.say("你是不是忘了什么？", Mood::Grumpy, now, 5.0);
                return None;
            }
        }
        if now >= self.next_antic && self.config.level != MischiefLevel::Calm {
            let antics = [
                ("我决定认真上班……明天开始。", Mood::Sleep),
                ("这里地板不平，不是我笨。", Mood::Dizzy),
                ("进度 99%……脑袋加载失败。", Mood::Grumpy),
                ("我刚钓到一只袜子！", Mood::Play),
                ("没偷吃。真的。", Mood::Happy),
                ("巡视领地，请勿打扰。", Mood::Walk),
            ];
            let choice = antics[self.random() as usize % antics.len()];
            let delay = 45.0 + (self.random() % 60) as f64;
            self.next_antic = now + delay;
            self.say(choice.0, choice.1, now, 5.0);
        }
        None
    }

    pub fn begin_chase(&mut self, now: f64) {
        self.say("来追你啦！双击我就停下。", Mood::Chase, now, 10.0);
    }

    pub fn begin_hide(&mut self, now: f64) {
        self.hidden = true;
        self.effect_until = now + self.config.icon_hide_seconds as f64;
        self.say(
            "图标在我手里！双击我就归还。",
            Mood::Hiding,
            now,
            self.config.icon_hide_seconds as f64,
        );
    }

    pub fn frame_interval(&self, now: f64) -> std::time::Duration {
        let fps = if self.quiet(now) {
            1.0
        } else if self.config.low_power {
            4.0
        } else if matches!(self.mood, Mood::Idle | Mood::Sleep | Mood::Grumpy) {
            2.0
        } else {
            12.0
        };
        std::time::Duration::from_secs_f64(1.0 / fps)
    }

    pub fn frame_interval_for_skin(
        &self,
        now: f64,
        animation: Option<std::time::Duration>,
    ) -> std::time::Duration {
        let interval = self.frame_interval(now);
        if self.quiet(now) || self.config.low_power {
            return interval;
        }
        animation.map_or(interval, |animation| {
            interval.min(animation.max(std::time::Duration::from_secs_f64(1.0 / 30.0)))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn engine() -> Engine {
        Engine::new(
            Config {
                neglect_seconds: 30,
                ..Config::default()
            },
            0.0,
            123,
        )
    }
    #[test]
    fn defaults_never_hide_or_chase() {
        let mut pet = engine();
        for time in 0..10000 {
            assert_ne!(pet.tick(time as f64, true), Some(Effect::HideIcons));
            assert_ne!(pet.mood, Mood::Chase);
        }
    }
    #[test]
    fn escalation_and_restore_are_bounded() {
        let mut pet = engine();
        pet.config.level = MischiefLevel::Devil;
        pet.config.allow_cursor_chase = true;
        pet.config.allow_hide_icons = true;
        pet.config.hide_icons_consent_version = 1;
        pet.tick(31.0, true);
        assert_eq!(pet.stage, 1);
        pet.tick(61.0, true);
        assert_eq!(pet.mood, Mood::Grumpy);
        assert_eq!(pet.tick(91.0, true), Some(Effect::HideIcons));
        assert_eq!(pet.tick(107.0, true), Some(Effect::RestoreIcons));
        assert_eq!(pet.tick(120.0, true), None);
        assert!(!pet.hidden);
    }
    #[test]
    fn neglect_selects_one_prank_for_the_foreground_without_switching_later() {
        for desktop_active in [true, false] {
            let mut pet = engine();
            pet.config.level = MischiefLevel::Devil;
            pet.config.allow_hide_icons = true;
            pet.config.hide_icons_consent_version = 1;
            pet.config.allow_cursor_chase = true;
            pet.next_antic = f64::INFINITY;
            pet.tick(31.0, desktop_active);
            pet.tick(61.0, desktop_active);
            assert!(!pet.mischief_active());
            assert_eq!(
                pet.tick(91.0, desktop_active),
                desktop_active.then_some(Effect::HideIcons)
            );
            assert_eq!(pet.stage, 3);
            assert_eq!(pet.hidden, desktop_active);
            assert_eq!(pet.mood == Mood::Chase, !desktop_active);
            assert_eq!(pet.tick(92.0, !desktop_active), None);
            assert_eq!(pet.hidden, desktop_active);
            assert_eq!(pet.mood == Mood::Chase, !desktop_active);
            assert_eq!(
                pet.tick(130.0, !desktop_active),
                desktop_active.then_some(Effect::RestoreIcons)
            );
            assert!(!pet.mischief_active());
            assert_eq!(pet.tick(400.0, true), None);
            assert!(!pet.mischief_active());
        }
    }

    #[test]
    fn chase_fallback_obeys_cursor_consent_in_playful_and_devil_modes() {
        for level in [MischiefLevel::Playful, MischiefLevel::Devil] {
            for allowed in [false, true] {
                let mut pet = engine();
                pet.config.level = level;
                pet.config.allow_cursor_chase = allowed;
                pet.next_antic = f64::INFINITY;
                assert_eq!(pet.tick(91.0, false), None);
                assert_eq!(pet.mood == Mood::Chase, allowed);
                assert!(!pet.hidden);
            }
        }
    }

    #[test]
    fn stop_mischief_interaction_ends_both_pranks_and_resets_neglect() {
        for hiding in [true, false] {
            let mut pet = engine();
            pet.config.level = MischiefLevel::Devil;
            pet.config.allow_hide_icons = true;
            pet.config.hide_icons_consent_version = 1;
            pet.config.allow_cursor_chase = true;
            if hiding {
                pet.begin_hide(91.0);
            } else {
                pet.begin_chase(91.0);
            }
            pet.stage = 3;
            assert_eq!(
                pet.interact(Interaction::StopMischief, 92.0),
                hiding.then_some(Effect::RestoreIcons)
            );
            assert!(!pet.mischief_active());
            assert_eq!(pet.stage, 0);
            assert_eq!(pet.last_interaction, 92.0);
            assert_eq!(pet.tick(93.0, true), None);
            assert_eq!(pet.tick(100.0, false), None);
            assert!(!pet.mischief_active());
            assert!(!pet.quiet(100.0));
        }
    }

    #[test]
    fn chase_still_times_out_without_a_double_click() {
        let mut pet = engine();
        pet.config.allow_cursor_chase = true;
        pet.next_antic = f64::INFINITY;
        pet.begin_chase(1.0);
        pet.tick(10.9, false);
        assert_eq!(pet.mood, Mood::Chase);
        pet.tick(11.0, false);
        assert!(!pet.mischief_active());
    }

    #[test]
    fn desktop_hide_still_requires_consent_and_devil_mode() {
        for (level, allowed, version, quiet) in [
            (MischiefLevel::Calm, true, 1, false),
            (MischiefLevel::Playful, true, 1, false),
            (MischiefLevel::Devil, false, 1, false),
            (MischiefLevel::Devil, true, 0, false),
            (MischiefLevel::Devil, true, 1, true),
        ] {
            let mut pet = engine();
            pet.config.level = level;
            pet.config.allow_hide_icons = allowed;
            pet.config.hide_icons_consent_version = version;
            pet.config.quiet = quiet;
            pet.next_antic = f64::INFINITY;
            assert_ne!(pet.tick(91.0, true), Some(Effect::HideIcons));
            assert!(!pet.hidden);
            assert_ne!(pet.mood, Mood::Chase);
        }
    }

    #[test]
    fn interaction_returns_icons_without_a_minigame() {
        let mut pet = engine();
        pet.begin_hide(0.0);
        assert_eq!(
            pet.interact(Interaction::Pet, 2.0),
            Some(Effect::RestoreIcons)
        );
        assert_eq!(pet.stage, 0);
    }
    #[test]
    fn quiet_prevents_mischief_and_cancels_existing_effect() {
        let mut pet = engine();
        pet.begin_hide(0.0);
        pet.config.quiet = true;
        assert_eq!(pet.tick(1.0, true), Some(Effect::RestoreIcons));
        for time in 2..500 {
            assert_eq!(pet.tick(time as f64, true), None);
        }
        assert_eq!(pet.mood, Mood::Sleep);
        assert_eq!(pet.frame_interval(500.0).as_secs(), 1);
    }
    #[test]
    fn busy_has_thirty_minute_cooldown() {
        let mut pet = engine();
        pet.interact(Interaction::Busy, 100.0);
        assert!(pet.quiet(1899.0));
        assert!(!pet.quiet(1901.0));
        pet.interact(Interaction::Wake, 200.0);
        assert!(!pet.quiet(200.0));
    }
    #[test]
    fn disable_consent_restores_and_memory_is_bounded() {
        let mut pet = engine();
        pet.begin_hide(0.0);
        assert_eq!(pet.tick(1.0, true), Some(Effect::RestoreIcons));
        for count in 0..100 {
            pet.interact(Interaction::Feed, count as f64);
        }
        assert_eq!(pet.config.affinity, 100);
        assert!(pet.history.len() <= 30);
    }

    #[test]
    fn revoking_cursor_consent_ends_chase_immediately() {
        let mut pet = engine();
        pet.config.allow_cursor_chase = true;
        pet.say("别看那里，看我！", Mood::Chase, 0.0, 10.0);
        pet.config.allow_cursor_chase = false;
        pet.tick(1.0, true);
        assert_ne!(pet.mood, Mood::Chase);
    }

    #[test]
    fn low_power_applies_to_chase_and_all_active_animations() {
        let mut pet = engine();
        pet.config.low_power = true;
        for mood in [
            Mood::Chase,
            Mood::Walk,
            Mood::Play,
            Mood::Happy,
            Mood::Dragged,
        ] {
            pet.mood = mood;
            assert_eq!(pet.frame_interval(0.0).as_millis(), 250);
        }
    }

    #[test]
    fn animated_skins_refresh_faster_but_obey_quiet_and_power_budgets() {
        use std::time::Duration;
        let mut pet = engine();
        let frame = Duration::from_secs_f64(1.0 / 24.0);
        let animation = Some(frame);
        for mood in [
            Mood::Idle,
            Mood::Sleep,
            Mood::Happy,
            Mood::Walk,
            Mood::Chase,
        ] {
            pet.mood = mood;
            assert_eq!(
                pet.frame_interval_for_skin(0.0, None),
                pet.frame_interval(0.0)
            );
            assert_eq!(pet.frame_interval_for_skin(0.0, animation), frame);
            assert_eq!(
                pet.frame_interval_for_skin(0.0, Some(Duration::ZERO)),
                Duration::from_secs_f64(1.0 / 30.0)
            );
            pet.config.low_power = true;
            assert_eq!(
                pet.frame_interval_for_skin(0.0, animation),
                Duration::from_millis(250)
            );
            pet.config.quiet = true;
            assert_eq!(
                pet.frame_interval_for_skin(0.0, animation),
                Duration::from_secs(1)
            );
            pet.config.quiet = false;
            pet.config.low_power = false;
        }
        pet.interact(Interaction::Busy, 0.0);
        assert_eq!(
            pet.frame_interval_for_skin(1.0, animation),
            Duration::from_secs(1)
        );
    }

    #[test]
    fn shutdown_stops_all_behaviors_without_changing_saved_preferences() {
        for mood in [
            Mood::Idle,
            Mood::Happy,
            Mood::Grumpy,
            Mood::Sleep,
            Mood::Dragged,
            Mood::Play,
            Mood::Chase,
            Mood::Hiding,
            Mood::Dizzy,
            Mood::Walk,
        ] {
            let mut pet = engine();
            pet.config.level = MischiefLevel::Devil;
            pet.config.allow_cursor_chase = true;
            pet.config.allow_hide_icons = true;
            pet.config.hide_icons_consent_version = 1;
            pet.begin_hide(90.0);
            pet.mood = mood;
            pet.stage = 3;
            let saved = serde_json::to_value(&pet.config).unwrap();

            pet.stop_mischief(91.0);
            pet.stop_mischief(92.0);

            assert!(!pet.hidden);
            assert!(pet.bubble.is_none());
            assert_eq!(pet.stage, 0);
            for time in [93.0, 200.0, 10000.0] {
                assert_eq!(pet.tick(time, true), None);
                assert_eq!(pet.mood, Mood::Sleep);
            }
            assert_eq!(serde_json::to_value(&pet.config).unwrap(), saved);
        }
    }
}
