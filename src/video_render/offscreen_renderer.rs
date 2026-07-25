use std::sync::Arc;
use egui::Pos2;
use vulkano::{
    buffer::{Buffer, BufferCreateInfo, BufferUsage, Subbuffer},
    command_buffer::{allocator::StandardCommandBufferAllocator, AutoCommandBufferBuilder, CommandBufferUsage, CopyImageToBufferInfo},
    device::{physical::PhysicalDeviceType, Device, DeviceCreateInfo, Queue, QueueCreateInfo, QueueFlags},
    format::Format,
    image::{view::ImageView, Image, ImageCreateInfo, ImageType, ImageUsage},
    instance::{Instance, InstanceCreateFlags, InstanceCreateInfo},
    memory::allocator::{AllocationCreateInfo, MemoryTypeFilter, StandardMemoryAllocator},
    sync::{self, GpuFuture}, VulkanLibrary,
};
use crate::{
    gui::window::{keyboard::draw_keyboard, keyboard_layout::KeyboardLayout, scene::{note_list_system::NoteRenderer, pie_system::PieRenderer}, stats::{draw_stats_panel, GuiMidiStats, NpsCounter}},
    midi::{MIDIFileBase, MIDIFileUnion}, settings::WasabiSettings, video_render::{egui_render_pass::EguiRenderer, RenderConfig},
};
use crate::gui::window::render_state::ParseMode;

enum SceneRenderer {
    Note(NoteRenderer),
    Pie(PieRenderer),
}

pub struct OffscreenRenderer {
    device: Arc<Device>, queue: Arc<Queue>,
    cb_allocator: Arc<StandardCommandBufferAllocator>,
    egui_renderer: EguiRenderer,
    scene_renderer: SceneRenderer,
    keyboard_layout: KeyboardLayout,
    stats: GuiMidiStats,
    nps_counter: NpsCounter,
    ppp: f32,
    final_image: Arc<ImageView>,
    depth_buffer: Arc<ImageView>,
    staging_buffer: Subbuffer<[u8]>,
    width: u32,
    keyboard_height: f32,
    notes_height: f32,
    viewport: vulkano::pipeline::graphics::viewport::Viewport,
}

impl OffscreenRenderer {
    pub fn new(config: &RenderConfig) -> Result<Self, String> {
        let (width, height) = config.resolution.dimensions();
        let lib = VulkanLibrary::new().map_err(|e| e.to_string())?;
        let inst = Instance::new(lib, InstanceCreateInfo { flags: InstanceCreateFlags::ENUMERATE_PORTABILITY, ..Default::default() }).map_err(|e| e.to_string())?;
        let p_dev = inst.enumerate_physical_devices().unwrap().min_by_key(|p| match p.properties().device_type {
            PhysicalDeviceType::DiscreteGpu => 0, PhysicalDeviceType::IntegratedGpu => 1, _ => 5,
        }).ok_or("no device")?;

        let q_fam = p_dev.queue_family_properties().iter().position(|q| q.queue_flags.contains(QueueFlags::GRAPHICS)).ok_or("no queue")? as u32;
        let (device, mut queues) = Device::new(p_dev, DeviceCreateInfo {
            enabled_features: vulkano::device::DeviceFeatures { geometry_shader: true, ..vulkano::device::DeviceFeatures::empty() },
            queue_create_infos: vec![QueueCreateInfo { queue_family_index: q_fam, ..Default::default() }], ..Default::default()
        }).map_err(|e| e.to_string())?;
        let queue = queues.next().unwrap();
        let alloc = Arc::new(StandardMemoryAllocator::new_default(device.clone()));
        let format = Format::B8G8R8A8_SRGB;

        let final_image = ImageView::new_default(Image::new(alloc.clone(), ImageCreateInfo { image_type: ImageType::Dim2d, format, extent: [width, height, 1], usage: ImageUsage::COLOR_ATTACHMENT | ImageUsage::TRANSFER_SRC | ImageUsage::SAMPLED, ..Default::default() }, Default::default()).unwrap()).unwrap();
        let depth_buffer = ImageView::new_default(Image::new(alloc.clone(), ImageCreateInfo { image_type: ImageType::Dim2d, format: Format::D16_UNORM, extent: [width, height, 1], usage: ImageUsage::DEPTH_STENCIL_ATTACHMENT, ..Default::default() }, Default::default()).unwrap()).unwrap();
        let staging_buffer = Buffer::new_slice::<u8>(alloc, BufferCreateInfo { usage: BufferUsage::TRANSFER_DST, ..Default::default() }, AllocationCreateInfo { memory_type_filter: MemoryTypeFilter::HOST_RANDOM_ACCESS, ..Default::default() }, (width * height * 4) as u64).unwrap();

        let ppp = ((height as f32 / 1080.0) * 1.5).max(1.0);
        
        let egui_renderer = EguiRenderer::new(device.clone(), queue.clone(), format, width, height, ppp);
        let scene_renderer = if config.parse_mode == ParseMode::Pie {
            SceneRenderer::Pie(PieRenderer::new(device.clone(), queue.clone(), format))
        } else {
            SceneRenderer::Note(NoteRenderer::new(device.clone(), queue.clone(), format))
        };

        let keyboard_height = height as f32 * 0.15;
        let notes_height = height as f32 - keyboard_height;
        let viewport = vulkano::pipeline::graphics::viewport::Viewport {
            offset: [0.0, 0.0],
            extent: [width as f32, notes_height],
            depth_range: 0.0..=1.0,
        };

        Ok(Self {
            device: device.clone(), queue: queue.clone(),
            cb_allocator: Arc::new(StandardCommandBufferAllocator::new(device.clone(), Default::default())),
            egui_renderer,
            scene_renderer,
            keyboard_layout: KeyboardLayout::new(&Default::default()),
            stats: GuiMidiStats::default(), nps_counter: NpsCounter::default(), ppp, final_image, depth_buffer, staging_buffer,
            width, keyboard_height, notes_height, viewport,
        })
    }

    pub fn render_frame_into(&mut self, midi_union: &mut MIDIFileUnion, range: f32, settings: &WasabiSettings, time: f64, consume: impl FnOnce(&[u8]) -> Result<(), String>) -> Result<(), String> {
        let key_view = self.keyboard_layout.get_view_for_keys(*settings.scene.key_range.start() as usize, *settings.scene.key_range.end() as usize);
        let bg = settings.scene.bg_color;

        let srgb_to_linear = |s: f32| if s <= 0.04045 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) };
        let bg_color = Some([
            srgb_to_linear(bg.r() as f32 / 255.0),
            srgb_to_linear(bg.g() as f32 / 255.0),
            srgb_to_linear(bg.b() as f32 / 255.0),
            1.0
        ]);

        let res = match (&mut *midi_union, &mut self.scene_renderer) {
            (MIDIFileUnion::Pie(m), SceneRenderer::Pie(r)) => r.draw(&key_view, self.final_image.clone(), m, range as f64, bg_color, Some(self.viewport.clone())),
            (MIDIFileUnion::Live(m), SceneRenderer::Note(r)) => r.draw(&key_view, self.final_image.clone(), m, range as f64, bg_color, Some(self.viewport.clone())),
            (MIDIFileUnion::InRam(m), SceneRenderer::Note(r)) => r.draw(&key_view, self.final_image.clone(), m, range as f64, bg_color, Some(self.viewport.clone())),
            _ => return Err("Mismatched renderer and MIDI mode".into()),
        };
        
        self.stats.notes_on_screen = res.notes_rendered;
        self.stats.polyphony = res.polyphony;
        if let Some(len) = midi_union.midi_length() { self.stats.time_total = len; }
        self.stats.time_passed = time;
        self.stats.note_stats = midi_union.stats();
        self.nps_counter.tick(self.stats.note_stats.passed_notes.unwrap_or(0) as i64);
        self.stats.nps = self.nps_counter.read();

        self.egui_renderer.begin_frame(time);
        let ctx = self.egui_renderer.context();
        let (w_p, k_h_p, n_h_p) = (self.width as f32 / self.ppp, self.keyboard_height / self.ppp, self.notes_height / self.ppp);

        egui::Area::new("k".into()).fixed_pos(Pos2::new(0.0, n_h_p)).show(ctx, |ui| {
             ui.allocate_ui(egui::Vec2::new(w_p, k_h_p), |ui| {
                 draw_keyboard(ui, &key_view, &res.key_colors, &settings.scene.bar_color);
             });
        });
        draw_stats_panel(ctx, Pos2::new(10.0, 10.0), &self.stats, settings, true);

        let mut builder = AutoCommandBufferBuilder::primary(self.cb_allocator.clone(), self.queue.queue_family_index(), CommandBufferUsage::OneTimeSubmit).map_err(|e| e.to_string())?;
        self.egui_renderer.end_frame(&mut builder, self.final_image.clone(), self.depth_buffer.clone());
        
        builder.copy_image_to_buffer(CopyImageToBufferInfo::image_buffer(self.final_image.image().clone(), self.staging_buffer.clone())).unwrap();
        let future = sync::now(self.device.clone()).then_execute(self.queue.clone(), builder.build().unwrap()).unwrap().then_signal_fence_and_flush().unwrap();
        future.wait(None).unwrap();
        
        let content = self.staging_buffer.read().unwrap();
        consume(&content)
    }
}
