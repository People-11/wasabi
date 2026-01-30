//! Offscreen renderer for video frame generation
//!
//! This module provides GPU rendering capabilities without a window,
//! allowing MIDI visualization to be rendered directly to buffers for video encoding.

use std::sync::Arc;
use std::collections::VecDeque;
use crate::gui::window::stats::GuiMidiStats;

use vulkano::{
    buffer::{Buffer, BufferCreateInfo, BufferUsage, Subbuffer},
    command_buffer::{
        allocator::StandardCommandBufferAllocator, AutoCommandBufferBuilder, CommandBufferUsage,
        CopyImageToBufferInfo,
    },
    device::{
        physical::PhysicalDeviceType, Device, DeviceCreateInfo, DeviceExtensions, DeviceFeatures,
        Queue, QueueCreateInfo, QueueFlags,
    },
    format::Format,
    image::{view::ImageView, Image, ImageCreateInfo, ImageUsage},
    instance::{Instance, InstanceCreateInfo},
    memory::allocator::{AllocationCreateInfo, MemoryTypeFilter, StandardMemoryAllocator},
    sync::{self, GpuFuture},
    VulkanLibrary,
};

use crate::gui::window::keyboard_layout::{KeyboardLayout, KeyboardParams};
use crate::gui::window::scene::note_list_system::NoteRenderer;
use crate::midi::MIDIFile;
use crate::settings::WasabiSettings;

/// Offscreen renderer for generating video frames
pub struct OffscreenRenderer {
    device: Arc<Device>,
    queue: Arc<Queue>,
    cb_allocator: Arc<StandardCommandBufferAllocator>,
    
    // Render target
    render_image: Arc<ImageView>,
    staging_buffer: Subbuffer<[u8]>,
    
    // Note renderer
    note_renderer: NoteRenderer,
    
    // Keyboard layout
    keyboard_layout: KeyboardLayout,
    
    // Dimensions
    width: u32,
    height: u32,
    
    // NPS calculation history
    nps_history: VecDeque<(f64, u64)>,
}

impl OffscreenRenderer {
    /// Create a new offscreen renderer with the specified dimensions
    pub fn new(width: u32, height: u32) -> Result<Self, String> {
        // Initialize Vulkan without a window
        let library = VulkanLibrary::new()
            .map_err(|e| format!("Failed to load Vulkan library: {}", e))?;

        let instance = Instance::new(
            library,
            InstanceCreateInfo {
                ..Default::default()
            },
        )
        .map_err(|e| format!("Failed to create Vulkan instance: {}", e))?;

        // Find a suitable GPU (no surface support needed)
        let device_extensions = DeviceExtensions::empty();
        let features = DeviceFeatures {
            geometry_shader: true,
            ..DeviceFeatures::empty()
        };

        let (physical_device, queue_family_index) = instance
            .enumerate_physical_devices()
            .map_err(|e| format!("Failed to enumerate physical devices: {}", e))?
            .filter(|p| p.supported_features().geometry_shader)
            .filter_map(|p| {
                p.queue_family_properties()
                    .iter()
                    .enumerate()
                    .position(|(_, q)| q.queue_flags.contains(QueueFlags::GRAPHICS))
                    .map(|i| (p, i as u32))
            })
            .min_by_key(|(p, _)| match p.properties().device_type {
                PhysicalDeviceType::DiscreteGpu => 0,
                PhysicalDeviceType::IntegratedGpu => 1,
                PhysicalDeviceType::VirtualGpu => 2,
                PhysicalDeviceType::Cpu => 3,
                PhysicalDeviceType::Other => 4,
                _ => 5,
            })
            .ok_or("No suitable GPU found with geometry shader support")?;

        println!(
            "[OffscreenRenderer] Using device: {} (type: {:?})",
            physical_device.properties().device_name,
            physical_device.properties().device_type,
        );

        // Create device
        let (device, mut queues) = Device::new(
            physical_device,
            DeviceCreateInfo {
                enabled_extensions: device_extensions,
                enabled_features: features,
                queue_create_infos: vec![QueueCreateInfo {
                    queue_family_index,
                    ..Default::default()
                }],
                ..Default::default()
            },
        )
        .map_err(|e| format!("Failed to create device: {}", e))?;

        let queue = queues.next().ok_or("No queue available")?;
        let allocator = Arc::new(StandardMemoryAllocator::new_default(device.clone()));
        let cb_allocator = Arc::new(StandardCommandBufferAllocator::new(
            device.clone(),
            Default::default(),
        ));

        // Create render target (BGRA for FFmpeg compatibility)
        let format = Format::B8G8R8A8_UNORM;
        let render_image = ImageView::new_default(
            Image::new(
                allocator.clone(),
                ImageCreateInfo {
                    extent: [width, height, 1],
                    format,
                    usage: ImageUsage::COLOR_ATTACHMENT | ImageUsage::TRANSFER_SRC,
                    ..Default::default()
                },
                Default::default(),
            )
            .map_err(|e| format!("Failed to create render image: {}", e))?,
        )
        .map_err(|e| format!("Failed to create render image view: {}", e))?;

        // Create staging buffer for reading pixels
        let buffer_size = (width * height * 4) as u64;
        let staging_buffer = Buffer::new_slice::<u8>(
            allocator.clone(),
            BufferCreateInfo {
                usage: BufferUsage::TRANSFER_DST,
                ..Default::default()
            },
            AllocationCreateInfo {
                memory_type_filter: MemoryTypeFilter::HOST_RANDOM_ACCESS,
                ..Default::default()
            },
            buffer_size,
        )
        .map_err(|e| format!("Failed to create staging buffer: {}", e))?;

        // Create NoteRenderer using the new offscreen constructor
        let note_renderer = NoteRenderer::new_offscreen(
            device.clone(),
            queue.clone(),
            format,
        );

        // Create keyboard layout
        let keyboard_layout = KeyboardLayout::new(&KeyboardParams::default());

        Ok(Self {
            device,
            queue,
            cb_allocator,
            render_image,
            staging_buffer,
            note_renderer,
            keyboard_layout,
            width,
            height,
            nps_history: VecDeque::new(),
        })
    }


    /// Render a frame and return the pixel data (BGRA format)
    pub fn render_frame(
        &mut self,
        midi_file: &mut impl MIDIFile,
        view_range: f64,
        settings: &WasabiSettings,
        current_time: f64,
    ) -> Result<Vec<u8>, String> {
        // Get keyboard view directly to avoid borrow conflict
        let first_key = *settings.scene.key_range.start() as usize;
        let last_key = *settings.scene.key_range.end() as usize;
        let key_view = self.keyboard_layout.get_view_for_keys(first_key, last_key);

        // Calculate keyboard height (must match what we use for rendering keyboard later)
        let keyboard_height = (11.6 / key_view.visible_range.len() as f32 * self.width as f32)
            .min(self.height as f32 / 2.0);
        let notes_height = self.height as f32 - keyboard_height;
        
        // Adjust view_range to account for keyboard taking up part of the screen
        // The notes area is only notes_height / height of the full frame
        let adjusted_view_range = view_range * (notes_height as f64 / self.height as f64);

        // Get background color from settings
        let bg = settings.scene.bg_color;
        let bg_color = Some([
            bg.r() as f32 / 255.0,
            bg.g() as f32 / 255.0,
            bg.b() as f32 / 255.0,
            bg.a() as f32 / 255.0,
        ]);

        // Create viewport for notes (excluding keyboard area)
        let viewport = vulkano::pipeline::graphics::viewport::Viewport {
            offset: [0.0, 0.0],
            extent: [self.width as f32, notes_height],
            depth_range: 0.0..=1.0,
        };

        // Render notes to the image
        let result = self.note_renderer.draw(
            &key_view, 
            self.render_image.clone(), 
            midi_file, 
            adjusted_view_range,
            bg_color,
            Some(viewport),
        );

        // Copy image to staging buffer
        let mut builder = AutoCommandBufferBuilder::primary(
            self.cb_allocator.clone(),
            self.queue.queue_family_index(),
            CommandBufferUsage::OneTimeSubmit,
        )
        .map_err(|e| format!("Failed to create command buffer: {}", e))?;

        builder
            .copy_image_to_buffer(CopyImageToBufferInfo::image_buffer(
                self.render_image.image().clone(),
                self.staging_buffer.clone(),
            ))
            .map_err(|e| format!("Failed to copy image to buffer: {}", e))?;

        let command_buffer = builder
            .build()
            .map_err(|e| format!("Failed to build command buffer: {}", e))?;

        // Execute and wait
        let future = sync::now(self.device.clone())
            .then_execute(self.queue.clone(), command_buffer)
            .map_err(|e| format!("Failed to execute command buffer: {}", e))?
            .then_signal_fence_and_flush()
            .map_err(|e| format!("Failed to signal fence: {}", e))?;

        future
            .wait(None)
            .map_err(|e| format!("Failed to wait for fence: {}", e))?;

        // Read pixels from staging buffer
        let buffer_content = self
            .staging_buffer
            .read()
            .map_err(|e| format!("Failed to read staging buffer: {}", e))?;

        let mut frame = buffer_content.to_vec();

        // Get bar color from settings
        let bar = settings.scene.bar_color;
        let bar_color = [bar.b(), bar.g(), bar.r(), bar.a()]; // BGRA

        // Render keyboard on top of the notes (software rendering)
        super::keyboard_renderer::render_keyboard(
            &mut frame,
            self.width,
            self.height,
            keyboard_height as u32,
            &key_view,
            &result.key_colors,
            bar_color,
        );
        
        // Calculate NPS using history
        let file_stats = midi_file.stats();
        let total_passed = file_stats.passed_notes.unwrap_or(0);
        
        self.nps_history.push_back((current_time, total_passed));
        
        // Remove old entries (> 1.0s ago)
        while let Some(&(t, _)) = self.nps_history.front() {
            if current_time - t > 1.0 {
                self.nps_history.pop_front();
            } else {
                break;
            }
        }
        
        let nps = if let (Some(&(start_t, start_n)), Some(&(end_t, end_n))) = (self.nps_history.front(), self.nps_history.back()) {
             let dt = end_t - start_t;
             if dt > 0.1 { // Require at least 0.1s of data to show meaningful NPS
                  ((end_n - start_n) as f64 / dt).round() as u64
             } else {
                 0
             }
        } else {
             0
        };

        // Construct stats for overlay
        let mut stats = GuiMidiStats::empty();
        stats.set_rendered_note_count(result.notes_rendered);
        stats.set_polyphony(result.polyphony);

        // Render overlay
        super::overlay_renderer::draw_overlay(
            &mut frame,
            self.width, 
            self.height,
            midi_file,
            current_time,
            &stats,
            nps,
            settings
        );

        Ok(frame)
    }

}

