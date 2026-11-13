use super::cases;
use anyhow::{Context as _, Result, ensure};
use eframe::{
    egui_glow::Painter,
    glow::{self, HasContext as _},
};
use serde::Serialize;
use std::{sync::Arc, time::Instant};

#[derive(Serialize)]
pub(super) struct Native {
    opengl_vendor: String,
    opengl_renderer: String,
    opengl_version: String,
    framebuffer_width: u32,
    framebuffer_height: u32,
    hidden_window_checks: usize,
    ordinary_frame_loop_entered: bool,
    pixel_readback_performed: bool,
    pub cases: Vec<cases::Case>,
}

#[cfg(windows)]
fn require_hidden(cc: &eframe::CreationContext<'_>) -> Result<()> {
    use raw_window_handle::{HasWindowHandle as _, RawWindowHandle};
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetWindowThreadProcessId, IsWindowVisible};
    let RawWindowHandle::Win32(handle) = cc.window_handle()?.as_raw() else {
        anyhow::bail!("expected this process's Win32 window");
    };
    let hwnd = handle.hwnd.get() as windows_sys::Win32::Foundation::HWND;
    let mut process_id = 0;
    // This is the creation context's live HWND; query only, without input or visibility changes.
    let thread = unsafe { GetWindowThreadProcessId(hwnd, &mut process_id) };
    ensure!(
        thread != 0 && process_id == std::process::id(),
        "self-test native window ownership is unknown"
    );
    ensure!(
        unsafe { IsWindowVisible(hwnd) } == 0,
        "self-test window must stay hidden"
    );
    Ok(())
}

#[cfg(not(windows))]
fn require_hidden(_: &eframe::CreationContext<'_>) -> Result<()> {
    anyhow::bail!("native visibility checks require Windows")
}

pub(super) fn render(cc: &eframe::CreationContext<'_>) -> Result<Native> {
    require_hidden(cc)?;
    let gl = cc.gl.clone().context("native OpenGL context unavailable")?;
    check_error(&gl, "native context initialization")?;
    // eframe has made this context current before invoking the app creator.
    let (vendor, renderer, version) = unsafe {
        (
            gl.get_parameter_string(glow::VENDOR),
            gl.get_parameter_string(glow::RENDERER),
            gl.get_parameter_string(glow::VERSION),
        )
    };
    ensure!(
        !vendor.is_empty() && !renderer.is_empty() && !version.is_empty(),
        "OpenGL driver identity unavailable"
    );
    let mut target = Target::new(gl.clone(), cases::SIZE)?;
    let started = Instant::now();
    let mut checks = 1;
    let cases = cases::render(&cc.egui_ctx, |size, output, primitives| {
        ensure!(
            started.elapsed().as_secs() < 20,
            "offscreen render time limit exceeded"
        );
        require_hidden(cc)?;
        checks += 1;
        target.paint(size, output, primitives)?;
        require_hidden(cc)?;
        checks += 1;
        Ok(())
    })?;
    drop(target);
    check_error(&gl, "offscreen resource cleanup")?;
    require_hidden(cc)?;
    checks += 1;
    Ok(Native {
        opengl_vendor: vendor,
        opengl_renderer: renderer,
        opengl_version: version,
        framebuffer_width: cases::SIZE[0],
        framebuffer_height: cases::SIZE[1],
        hidden_window_checks: checks,
        ordinary_frame_loop_entered: false,
        pixel_readback_performed: false,
        cases,
    })
}

fn check_error(gl: &glow::Context, stage: &str) -> Result<()> {
    // Only called inside the native app creator while its GL context remains current.
    let error = unsafe { gl.get_error() };
    ensure!(
        error == glow::NO_ERROR,
        "OpenGL error 0x{error:04X} after {stage}"
    );
    Ok(())
}

struct Target {
    gl: Arc<glow::Context>,
    painter: Painter,
    framebuffer: Option<glow::NativeFramebuffer>,
    color: Option<glow::NativeRenderbuffer>,
}

impl Target {
    fn new(gl: Arc<glow::Context>, size: [u32; 2]) -> Result<Self> {
        let painter = Painter::new(gl.clone(), "", None, false)
            .map_err(|error| anyhow::anyhow!("offscreen painter: {error}"))?;
        let mut target = Self {
            gl,
            painter,
            framebuffer: None,
            color: None,
        };
        // All native handles are owned by target and deleted before the context can leave the creator.
        unsafe {
            let maximum = target.gl.get_parameter_i32(glow::MAX_RENDERBUFFER_SIZE);
            ensure!(
                size.iter()
                    .all(|&value| i32::try_from(value).is_ok_and(|value| value <= maximum)),
                "offscreen framebuffer exceeds the OpenGL renderbuffer limit"
            );
            target.framebuffer = Some(target.gl.create_framebuffer().map_err(anyhow::Error::msg)?);
            target.color = Some(
                target
                    .gl
                    .create_renderbuffer()
                    .map_err(anyhow::Error::msg)?,
            );
            target
                .gl
                .bind_framebuffer(glow::FRAMEBUFFER, target.framebuffer);
            target
                .gl
                .bind_renderbuffer(glow::RENDERBUFFER, target.color);
            target.gl.renderbuffer_storage(
                glow::RENDERBUFFER,
                glow::RGBA8,
                size[0] as i32,
                size[1] as i32,
            );
            target.gl.framebuffer_renderbuffer(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::RENDERBUFFER,
                target.color,
            );
            ensure!(
                target.gl.check_framebuffer_status(glow::FRAMEBUFFER) == glow::FRAMEBUFFER_COMPLETE,
                "offscreen framebuffer incomplete"
            );
        }
        check_error(&target.gl, "offscreen target creation")?;
        Ok(target)
    }

    fn paint(
        &mut self,
        size: [u32; 2],
        output: &eframe::egui::FullOutput,
        primitives: &[eframe::egui::ClippedPrimitive],
    ) -> Result<()> {
        unsafe {
            self.gl
                .bind_framebuffer(glow::FRAMEBUFFER, self.framebuffer);
            self.gl.disable(glow::SCISSOR_TEST);
            self.gl.clear_color(0.0, 0.0, 0.0, 0.0);
            self.gl.clear(glow::COLOR_BUFFER_BIT);
        }
        self.painter.paint_and_update_textures(
            size,
            output.pixels_per_point,
            primitives,
            &output.textures_delta,
        );
        unsafe {
            self.gl.finish();
            ensure!(
                self.gl.get_parameter_i32(glow::FRAMEBUFFER_BINDING)
                    == self
                        .framebuffer
                        .context("offscreen framebuffer missing")?
                        .0
                        .get() as i32,
                "painter left the offscreen framebuffer"
            );
        }
        check_error(&self.gl, "offscreen view painting")
    }
}

impl Drop for Target {
    fn drop(&mut self) {
        self.painter.destroy();
        // Drop runs before the creation context is released, including error/unwind paths.
        unsafe {
            self.gl.bind_framebuffer(glow::FRAMEBUFFER, None);
            self.gl.bind_renderbuffer(glow::RENDERBUFFER, None);
            if let Some(color) = self.color.take() {
                self.gl.delete_renderbuffer(color);
            }
            if let Some(framebuffer) = self.framebuffer.take() {
                self.gl.delete_framebuffer(framebuffer);
            }
        }
    }
}
