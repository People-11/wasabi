use std::sync::{Arc, OnceLock};

use egui::load::SizedTexture;
use vulkano::{
    buffer::{Buffer, BufferCreateInfo, BufferUsage},
    command_buffer::{
        allocator::StandardCommandBufferAllocator, AutoCommandBufferBuilder, BufferImageCopy,
        CommandBufferUsage, CopyBufferToImageInfo,
    },
    format::Format,
    image::{
        sampler::{SamplerAddressMode, SamplerCreateInfo},
        view::ImageView,
        Image, ImageCreateInfo, ImageSubresourceLayers, ImageUsage,
    },
    memory::allocator::{AllocationCreateInfo, MemoryTypeFilter, StandardMemoryAllocator},
    sync::{self, GpuFuture},
};

use super::GuiRenderer;

mod data {
    include!(concat!(env!("OUT_DIR"), "/ui_icons.rs"));
}

pub struct Icons {
    pub folder: SizedTexture,
    pub stop: SizedTexture,
    pub play: SizedTexture,
    pub pause: SizedTexture,
    pub options: SizedTexture,
    pub pin: SizedTexture,
    pub error: SizedTexture,
    pub warning: SizedTexture,
    pub logo: SizedTexture,
}

static ICONS: OnceLock<Icons> = OnceLock::new();

pub fn icons() -> &'static Icons {
    ICONS.get().expect("icons not loaded")
}

/// Uploads the icons pre-rendered by build.rs as mipmapped textures
pub fn load(renderer: &mut GuiRenderer) {
    let allocator = Arc::new(StandardMemoryAllocator::new_default(renderer.device.clone()));
    let cb_allocator = Arc::new(StandardCommandBufferAllocator::new(
        renderer.device.clone(),
        Default::default(),
    ));
    let mut builder = AutoCommandBufferBuilder::primary(
        cb_allocator,
        renderer.queue.queue_family_index(),
        CommandBufferUsage::OneTimeSubmit,
    )
    .unwrap();

    let mut upload = |(size, levels): (u32, &[&[u8]])| {
        let mut pixels = Vec::new();
        let mut regions = Vec::new();
        for (level, png) in levels.iter().enumerate() {
            let level_size = size >> level;
            regions.push(BufferImageCopy {
                buffer_offset: pixels.len() as u64,
                image_subresource: ImageSubresourceLayers {
                    mip_level: level as u32,
                    ..ImageSubresourceLayers::from_parameters(Format::R8G8B8A8_SRGB, 1)
                },
                image_extent: [level_size, level_size, 1],
                ..Default::default()
            });
            let image = image::load_from_memory_with_format(png, image::ImageFormat::Png).unwrap();
            pixels.extend_from_slice(image.as_bytes());
        }

        let buffer = Buffer::from_iter(
            allocator.clone(),
            BufferCreateInfo {
                usage: BufferUsage::TRANSFER_SRC,
                ..Default::default()
            },
            AllocationCreateInfo {
                memory_type_filter: MemoryTypeFilter::HOST_SEQUENTIAL_WRITE,
                ..Default::default()
            },
            pixels,
        )
        .unwrap();
        let image = Image::new(
            allocator.clone(),
            ImageCreateInfo {
                format: Format::R8G8B8A8_SRGB,
                extent: [size, size, 1],
                mip_levels: levels.len() as u32,
                usage: ImageUsage::SAMPLED | ImageUsage::TRANSFER_DST,
                ..Default::default()
            },
            Default::default(),
        )
        .unwrap();
        builder
            .copy_buffer_to_image(CopyBufferToImageInfo {
                regions: regions.into(),
                ..CopyBufferToImageInfo::buffer_image(buffer, image.clone())
            })
            .unwrap();

        let id = renderer.gui.register_user_image_view(
            ImageView::new_default(image).unwrap(),
            SamplerCreateInfo {
                address_mode: [SamplerAddressMode::ClampToEdge; 3],
                // Lean on the next larger level (a mild bilinear downscale) rather than
                // blending in the smaller, blurrier one
                mip_lod_bias: -0.5,
                ..SamplerCreateInfo::simple_repeat_linear()
            },
        );
        // build.rs renders at 4x the largest size the icon is shown at
        SizedTexture::new(id, [size as f32 / 4.0; 2])
    };

    let icons = Icons {
        folder: upload(data::FOLDER),
        stop: upload(data::STOP),
        play: upload(data::PLAY),
        pause: upload(data::PAUSE),
        options: upload(data::OPTIONS),
        pin: upload(data::PIN),
        error: upload(data::ERROR),
        warning: upload(data::WARNING),
        logo: upload(data::LOGO),
    };

    sync::now(renderer.device.clone())
        .then_execute(renderer.queue.clone(), builder.build().unwrap())
        .unwrap()
        .then_signal_fence_and_flush()
        .unwrap()
        .wait(None)
        .unwrap();

    let _ = ICONS.set(icons);
}
