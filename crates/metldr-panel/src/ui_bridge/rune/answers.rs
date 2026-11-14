use super::{Activities, UiAnswerFrame, Updates, activities::AnswerMount};
use crate::ag_ui::{Activity, AgUiMessage};
use std::{
    cell::RefCell,
    collections::HashMap,
    rc::{Rc, Weak},
};
use wasm_bindgen::{JsCast, JsValue};

pub(super) struct Followup {
    pub text: String,
    id: String,
    frame: Weak<UiAnswerFrame>,
    initialization: String,
}

impl Followup {
    pub fn is_current(&self, activities: &Activities) -> bool {
        self.frame
            .upgrade()
            .zip(activities.answer_frame(&self.id))
            .is_some_and(|(offered, current)| {
                Rc::ptr_eq(&offered, &current)
                    && current
                        .initialize()
                        .is_ok_and(|binding| binding == self.initialization)
            })
    }
}

#[derive(Default)]
pub(super) struct Answers {
    mounts: RefCell<HashMap<String, Mounted>>,
}

struct Mounted {
    container: web_sys::Element,
    mount: AnswerMount,
    rect: ::rune::Live<[f32; 4]>,
    appearance: Option<(String, String, u32)>,
}

impl Drop for Mounted {
    fn drop(&mut self) {
        self.container.remove();
    }
}

impl Answers {
    pub fn sync(
        &self,
        activities: &Activities,
        messages: &[AgUiMessage],
        updates: &Rc<Updates>,
    ) -> Result<(), JsValue> {
        let mut mounts = self.mounts.borrow_mut();
        mounts.retain(|id, mounted| {
            activities
                .answer_frame(id)
                .is_some_and(|frame| mounted.mount.matches_controller(&frame))
        });
        for message in messages {
            let AgUiMessage::Activity {
                id,
                activity: Activity::Generated(_),
            } = message
            else {
                continue;
            };
            let Some(frame) = activities.answer_frame(id) else {
                continue;
            };
            if !frame.has_content() || mounts.contains_key(id) {
                continue;
            }
            let Some(rect) = activities.answer_rect(id) else {
                continue;
            };
            let Some(gate) = activities.answer_gate(id) else {
                continue;
            };
            let document = web_sys::window()
                .and_then(|window| window.document())
                .ok_or_else(|| crate::err("Document unavailable."))?;
            let container = document.create_element("div")?;
            container.set_attribute("data-rune-answer", id)?;
            document
                .body()
                .ok_or_else(|| crate::err("Document body unavailable."))?
                .append_child(&container)?;
            let weak_updates = Rc::downgrade(updates);
            let weak_frame = Rc::downgrade(&frame);
            let initialization = frame.initialize()?;
            let offered_id = id.clone();
            let followup = Rc::new(move |text| {
                if let Some(updates) = weak_updates.upgrade()
                    && !updates.closed.get()
                {
                    updates.presentation.borrow_mut().followup = Some(Followup {
                        text,
                        id: offered_id.clone(),
                        frame: weak_frame.clone(),
                        initialization: initialization.clone(),
                    });
                    updates.changed();
                }
            });
            let failure = activities.answer_failure(id, updates);
            let weak_updates = Rc::downgrade(updates);
            let wheel = Rc::new(move |dx: f32, dy: f32| {
                if weak_updates
                    .upgrade()
                    .is_none_or(|updates| updates.closed.get())
                {
                    return;
                }
                #[cfg(target_arch = "wasm32")]
                super::HOST.with(|host| {
                    if let Ok(host) = host.try_borrow()
                        && let Some(handle) = host.as_ref()
                    {
                        handle.relay_wheel("messages", dx, dy);
                    }
                });
                #[cfg(not(target_arch = "wasm32"))]
                let _ = (dx, dy);
            });
            let mounted = AnswerMount::attach(
                &container,
                frame,
                followup,
                failure.clone(),
                activities.answer_height(id, updates),
                gate,
                wheel,
            );
            match mounted {
                Ok(mount) => {
                    mounts.insert(
                        id.clone(),
                        Mounted {
                            container,
                            mount,
                            rect,
                            appearance: None,
                        },
                    );
                }
                Err(error) => {
                    container.remove();
                    failure();
                    return Err(error);
                }
            }
        }
        for (id, mounted) in mounts.iter() {
            if let Err(error) = mounted.mount.pump() {
                activities.answer_failure(id, updates)();
                return Err(error);
            }
        }
        Ok(())
    }

    pub fn consume_followup(&self, followup: &Followup) -> Result<(), JsValue> {
        let mounts = self.mounts.borrow();
        let mounted = mounts
            .get(&followup.id)
            .ok_or_else(|| crate::err("Answer unavailable."))?;
        let frame = followup
            .frame
            .upgrade()
            .ok_or_else(|| crate::err("Answer unavailable."))?;
        if !mounted.mount.matches_controller(&frame) {
            return Err(crate::err("Answer changed."));
        }
        mounted.mount.consume_followup()
    }

    pub fn focus(&self, id: &str) -> Result<(), JsValue> {
        let mounts = self.mounts.borrow();
        let mounted = mounts
            .get(id)
            .ok_or_else(|| crate::err("Answer unavailable."))?;
        mounted
            .container
            .first_element_child()
            .ok_or_else(|| crate::err("Answer unavailable."))?
            .dyn_into::<web_sys::HtmlElement>()?
            .focus()
    }

    pub fn drawn(
        &self,
        clip: [f32; 4],
        updates: &Rc<Updates>,
        activities: &Activities,
        palette: &super::preferences::Palette,
        typography: super::preferences::Typography,
    ) {
        for (id, mounted) in self.mounts.borrow_mut().iter_mut() {
            let appearance = (
                palette.surface.hex(),
                palette.ink.hex(),
                typography.scale.to_bits(),
            );
            let result = (|| {
                if mounted.appearance.as_ref() != Some(&appearance) {
                    let host = mounted
                        .container
                        .dyn_ref::<web_sys::HtmlElement>()
                        .ok_or_else(|| crate::err("Answer host unavailable."))?;
                    let style = host.style();
                    style.set_property("background-color", &appearance.0)?;
                    style.set_property("color", &appearance.1)?;
                    style.set_property("font-family", "Inter, sans-serif")?;
                    style.set_property("font-size", &format!("{}px", 14.0 * typography.scale))?;
                    style.set_property("--chat-text-scale", &typography.scale.to_string())?;
                    mounted.appearance = Some(appearance);
                }
                mounted.mount.position(mounted.rect.get(), clip)
            })();
            if result.is_err() {
                activities.answer_failure(id, updates)();
            }
        }
    }
}
