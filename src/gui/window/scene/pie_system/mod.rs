use std::sync::Arc;
use vulkano::command_buffer::CopyBufferInfo;

use bytemuck::{Pod, Zeroable};
use vulkano::{
    buffer::{Buffer, BufferCreateInfo, BufferUsage, Subbuffer},
    command_buffer::{
        allocator::{StandardCommandBufferAllocator, StandardCommandBufferAllocatorCreateInfo},
        AutoCommandBufferBuilder, CommandBufferUsage, RenderPassBeginInfo, SubpassBeginInfo,
        SubpassContents,
    },
    descriptor_set::{
        allocator::{StandardDescriptorSetAllocator, StandardDescriptorSetAllocatorCreateInfo},
        DescriptorSet, WriteDescriptorSet,
    },
    device::{Device, Queue},
    format::{ClearValue, Format},
    image::{view::ImageView, Image, ImageCreateInfo, ImageUsage},
    memory::allocator::{AllocationCreateInfo, MemoryTypeFilter, StandardMemoryAllocator},
    pipeline::{
        graphics::{
            color_blend::{ColorBlendAttachmentState, ColorBlendState},
            depth_stencil::{DepthState, DepthStencilState},
            input_assembly::{InputAssemblyState, PrimitiveTopology},
            multisample::MultisampleState,
            rasterization::RasterizationState,
            vertex_input::{Vertex, VertexDefinition},
            viewport::Viewport,
            GraphicsPipelineCreateInfo,
        },
        layout::PipelineDescriptorSetLayoutCreateInfo,
        DynamicState, GraphicsPipeline, Pipeline, PipelineBindPoint, PipelineLayout,
        PipelineShaderStageCreateInfo,
    },
    render_pass::{Framebuffer, FramebufferCreateInfo, RenderPass, Subpass},
    sync::{self, GpuFuture},
};

use crate::{
    gui::window::keyboard_layout::KeyboardView,
    midi::PieMIDIFile,
};

use super::RenderResultData;

/// Tree values per staging upload (128MB)
const STAGING_LEN: usize = 32 * 1024 * 1024;

#[derive(Default, Debug, Copy, Clone, Zeroable, Pod, Vertex)]
#[repr(C)]
struct PieNoteColumn {
    #[format(R32_SFLOAT)]
    left: f32,
    #[format(R32_SFLOAT)]
    right: f32,
    #[format(R32_SINT)]
    start: i32,
    #[format(R32_SINT)]
    end: i32,
    #[format(R32_SINT)]
    tree_offset: i32,
    #[format(R32_SINT)]
    border_width: i32,
}

struct PieBatch {
    _buffer: Subbuffer<[i32]>,
    start_key: usize,
    end_key: usize,
    base_offset: usize,
    descriptor_set: Arc<DescriptorSet>,
    /// Replaced (never written in place) since in-flight frames may still read the old one
    vbo: Option<Subbuffer<[PieNoteColumn]>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PieVboCacheSignature {
    notes_hash: u64,
    border_width: i32,
}

struct CachedFramebuffer {
    color_attachment: Arc<ImageView>,
    framebuffer: Arc<Framebuffer>,
}

pub struct PieRenderer {
    gfx_queue: Arc<Queue>,
    batches: Vec<PieBatch>,
    pipeline_clear: Arc<GraphicsPipeline>,
    render_pass_clear: Arc<RenderPass>,
    allocator: Arc<StandardMemoryAllocator>,
    depth_buffer: Arc<ImageView>,
    cb_allocator: Arc<StandardCommandBufferAllocator>,
    sd_allocator: Arc<StandardDescriptorSetAllocator>,
    vbo_cache_signature: Option<PieVboCacheSignature>,
    framebuffer_cache: Vec<CachedFramebuffer>,
}

impl PieRenderer {
    pub fn new(device: Arc<Device>, queue: Arc<Queue>, format: Format) -> PieRenderer {
        let allocator = Arc::new(StandardMemoryAllocator::new_default(device.clone()));

        let render_pass_clear = vulkano::ordered_passes_renderpass!(device.clone(),
            attachments: {
                final_color: { format: format, samples: 1, load_op: Clear, store_op: Store },
                depth: { format: Format::D16_UNORM, samples: 1, load_op: Clear, store_op: Store }
            },
            passes: [{ color: [final_color], depth_stencil: {depth}, input: [] }]
        )
        .unwrap();

        let depth_buffer = ImageView::new_default(
            Image::new(
                allocator.clone(),
                ImageCreateInfo {
                    extent: [1, 1, 1],
                    format: Format::D16_UNORM,
                    usage: ImageUsage::SAMPLED | ImageUsage::DEPTH_STENCIL_ATTACHMENT,
                    ..Default::default()
                },
                Default::default(),
            )
            .unwrap(),
        )
        .unwrap();

        let vs = vs::load(device.clone())
            .unwrap()
            .entry_point("main")
            .unwrap();
        let fs = fs::load(device.clone())
            .unwrap()
            .entry_point("main")
            .unwrap();
        let gs = gs::load(device.clone())
            .unwrap()
            .entry_point("main")
            .unwrap();

        let vertex_input_state = PieNoteColumn::per_vertex().definition(&vs).unwrap();
        let stages = [
            PipelineShaderStageCreateInfo::new(vs),
            PipelineShaderStageCreateInfo::new(fs),
            PipelineShaderStageCreateInfo::new(gs),
        ];
        let layout = PipelineLayout::new(
            device.clone(),
            PipelineDescriptorSetLayoutCreateInfo::from_stages(&stages)
                .into_pipeline_layout_create_info(device.clone())
                .unwrap(),
        )
        .unwrap();
        let subpass = Subpass::from(render_pass_clear.clone(), 0).unwrap();

        let pipeline_clear = GraphicsPipeline::new(
            device.clone(),
            None,
            GraphicsPipelineCreateInfo {
                stages: stages.into_iter().collect(),
                vertex_input_state: Some(vertex_input_state),
                input_assembly_state: Some(InputAssemblyState {
                    topology: PrimitiveTopology::PointList,
                    ..Default::default()
                }),
                viewport_state: Some(Default::default()),
                dynamic_state: [DynamicState::Viewport].into_iter().collect(),
                rasterization_state: Some(RasterizationState::default()),
                multisample_state: Some(MultisampleState::default()),
                color_blend_state: Some(ColorBlendState::with_attachment_states(
                    subpass.num_color_attachments(),
                    ColorBlendAttachmentState::default(),
                )),
                depth_stencil_state: Some(DepthStencilState {
                    depth: Some(DepthState::simple()),
                    ..Default::default()
                }),
                subpass: Some(subpass.into()),
                ..GraphicsPipelineCreateInfo::layout(layout)
            },
        )
        .unwrap();

        PieRenderer {
            gfx_queue: queue,
            batches: vec![],
            pipeline_clear,
            render_pass_clear,
            depth_buffer,
            allocator,
            cb_allocator: StandardCommandBufferAllocator::new(
                device.clone(),
                StandardCommandBufferAllocatorCreateInfo::default(),
            )
            .into(),
            sd_allocator: StandardDescriptorSetAllocator::new(
                device.clone(),
                StandardDescriptorSetAllocatorCreateInfo::default(),
            )
            .into(),
            vbo_cache_signature: None,
            framebuffer_cache: vec![],
        }
    }

    fn vbo_cache_signature(key_view: &KeyboardView, border_width: i32) -> PieVboCacheSignature {
        let mut hash = 0xcbf29ce484222325u64;

        fn hash_u64(hash: &mut u64, value: u64) {
            *hash ^= value;
            *hash = hash.wrapping_mul(0x100000001b3);
        }

        hash_u64(&mut hash, key_view.visible_range.start as u64);
        hash_u64(&mut hash, key_view.visible_range.end as u64);

        for note in key_view.iter_all_notes() {
            hash_u64(&mut hash, note.left.to_bits() as u64);
            hash_u64(&mut hash, note.right.to_bits() as u64);
            hash_u64(&mut hash, note.black as u64);
        }

        PieVboCacheSignature {
            notes_hash: hash,
            border_width,
        }
    }

    fn update_vbo_cache(
        &mut self,
        key_view: &KeyboardView,
        midi_file: &PieMIDIFile,
        border_width: i32,
    ) {
        let signature = Self::vbo_cache_signature(key_view, border_width);
        if self.vbo_cache_signature == Some(signature) {
            return;
        }

        let flat_blocks = midi_file.flat_blocks();

        for batch in &mut self.batches {
            // Black keys first so their depth hides the white notes underneath
            let keys = (batch.start_key..batch.end_key).map(|i| (i, key_view.note(i)));
            let (black, white): (Vec<_>, Vec<_>) = keys.partition(|(_, key)| key.black);
            let columns = black.into_iter().chain(white).map(|(i, key)| PieNoteColumn {
                tree_offset: (flat_blocks.get_block_info(i).tree_offset - batch.base_offset) as i32,
                border_width,
                start: flat_blocks.start_time() as i32,
                end: flat_blocks.end_time() as i32,
                left: key.left,
                right: key.right,
            });
            let columns: Vec<_> = columns.collect();

            batch.vbo = Some(
                Buffer::from_iter(
                    self.allocator.clone(),
                    BufferCreateInfo {
                        usage: BufferUsage::VERTEX_BUFFER,
                        ..Default::default()
                    },
                    AllocationCreateInfo {
                        memory_type_filter: MemoryTypeFilter::HOST_SEQUENTIAL_WRITE,
                        ..Default::default()
                    },
                    columns,
                )
                .unwrap(),
            );
        }

        self.vbo_cache_signature = Some(signature);
    }

    /// Copies the first `len` values of `staging` into `dst` at `dst_offset`, blocking
    /// until done so the staging buffer can be refilled.
    fn copy_and_wait(
        &self,
        staging: &Subbuffer<[i32]>,
        dst: &Subbuffer<[i32]>,
        len: usize,
        dst_offset: usize,
    ) {
        let mut builder = AutoCommandBufferBuilder::primary(
            self.cb_allocator.clone(),
            self.gfx_queue.queue_family_index(),
            CommandBufferUsage::OneTimeSubmit,
        )
        .unwrap();
        builder
            .copy_buffer(CopyBufferInfo::buffers(
                staging.clone().slice(0..len as u64),
                dst.clone()
                    .slice(dst_offset as u64..(dst_offset + len) as u64),
            ))
            .unwrap();

        sync::now(self.gfx_queue.device().clone())
            .then_execute(self.gfx_queue.clone(), builder.build().unwrap())
            .unwrap()
            .then_signal_fence_and_flush()
            .unwrap()
            .wait(None)
            .unwrap();
    }

    fn get_or_create_framebuffer(&mut self, final_image: Arc<ImageView>) -> Arc<Framebuffer> {
        if let Some(entry) = self
            .framebuffer_cache
            .iter()
            .find(|entry| Arc::ptr_eq(&entry.color_attachment, &final_image))
        {
            return entry.framebuffer.clone();
        }

        let framebuffer = Framebuffer::new(
            self.render_pass_clear.clone(),
            FramebufferCreateInfo {
                attachments: vec![final_image.clone(), self.depth_buffer.clone()],
                ..Default::default()
            },
        )
        .unwrap();

        self.framebuffer_cache.push(CachedFramebuffer {
            color_attachment: final_image,
            framebuffer: framebuffer.clone(),
        });

        framebuffer
    }

    pub fn draw(
        &mut self,
        key_view: &KeyboardView,
        final_image: Arc<ImageView>,
        midi_file: &mut PieMIDIFile,
        view_range: f64,
        bg_color: Option<[f32; 4]>,
        viewport: Option<Viewport>,
        before: Box<dyn GpuFuture>,
    ) -> (RenderResultData, Box<dyn GpuFuture>) {
        let img_dims = final_image.image().extent();
        if self.depth_buffer.image().extent() != img_dims {
            self.depth_buffer = ImageView::new_default(
                Image::new(
                    self.allocator.clone(),
                    ImageCreateInfo {
                        extent: [img_dims[0], img_dims[1], 1],
                        format: Format::D16_UNORM,
                        usage: ImageUsage::SAMPLED | ImageUsage::DEPTH_STENCIL_ATTACHMENT,
                        ..Default::default()
                    },
                    Default::default(),
                )
                .unwrap(),
            )
            .unwrap();
            self.framebuffer_cache.clear();
        }

        // A file that still has its trees hasn't been uploaded yet (i.e. it's a new file).
        // The GPU keeps the only copy of them from here on.
        if let Some(trees) = midi_file.flat_blocks_mut().take_trees() {
            self.vbo_cache_signature = None;
            self.batches.clear();

            let flat_blocks = midi_file.flat_blocks();
            let mut current_batch_start = 0;
            let mut current_batch_size = 0;
            let mut current_start_offset = 0;
            let target_batch_size = self
                .gfx_queue
                .device()
                .physical_device()
                .properties()
                .max_storage_buffer_range as usize;

            let mut chunks = Vec::new();

            for i in 0..flat_blocks.len() {
                let info = flat_blocks.get_block_info(i);
                // 4 bytes per int
                let size_bytes = info.tree_len * 4;

                if current_batch_size + size_bytes > target_batch_size && current_batch_size > 0 {
                    chunks.push((
                        current_batch_start,
                        i,
                        current_start_offset,
                        info.tree_offset,
                    ));
                    current_batch_start = i;
                    current_batch_size = 0;
                    current_start_offset = info.tree_offset;
                }
                current_batch_size += size_bytes;
            }

            if current_batch_start < flat_blocks.len() {
                chunks.push((
                    current_batch_start,
                    flat_blocks.len(),
                    current_start_offset,
                    flat_blocks.total_len(),
                ));
            }

            // The trees are streamed through a fixed-size staging buffer instead of one as big
            // as the file. It gets its own allocator because vulkano's allocators keep their
            // memory blocks around for reuse, which would pin the staging memory forever.
            let staging_allocator = Arc::new(StandardMemoryAllocator::new_default(
                self.gfx_queue.device().clone(),
            ));
            let staging: Subbuffer<[i32]> = Buffer::new_slice(
                staging_allocator,
                BufferCreateInfo {
                    usage: BufferUsage::TRANSFER_SRC,
                    ..Default::default()
                },
                AllocationCreateInfo {
                    memory_type_filter: MemoryTypeFilter::HOST_SEQUENTIAL_WRITE,
                    ..Default::default()
                },
                STAGING_LEN.min(flat_blocks.total_len()) as u64,
            )
            .unwrap();
            let staging_len = staging.len() as usize;

            let pipeline_layout = self.pipeline_clear.layout();
            let desc_layout = pipeline_layout.set_layouts().first().unwrap();

            for (start_key, end_key, start_offset, end_offset) in chunks {
                let device_buffer: Subbuffer<[i32]> = Buffer::new_slice(
                    self.allocator.clone(),
                    BufferCreateInfo {
                        usage: BufferUsage::STORAGE_BUFFER | BufferUsage::TRANSFER_DST,
                        ..Default::default()
                    },
                    AllocationCreateInfo {
                        memory_type_filter: MemoryTypeFilter::PREFER_DEVICE,
                        ..Default::default()
                    },
                    (end_offset - start_offset) as u64,
                )
                .unwrap();

                let mut filled = 0;
                let mut uploaded = 0;
                for tree in &trees[start_key..end_key] {
                    let mut tree = &tree[..];
                    while !tree.is_empty() {
                        let n = tree.len().min(staging_len - filled);
                        staging.write().unwrap()[filled..filled + n].copy_from_slice(&tree[..n]);
                        filled += n;
                        tree = &tree[n..];

                        if filled == staging_len {
                            self.copy_and_wait(&staging, &device_buffer, filled, uploaded);
                            uploaded += filled;
                            filled = 0;
                        }
                    }
                }
                if filled > 0 {
                    self.copy_and_wait(&staging, &device_buffer, filled, uploaded);
                }

                let descriptor_set = DescriptorSet::new(
                    self.sd_allocator.clone(),
                    desc_layout.clone(),
                    [WriteDescriptorSet::buffer(0, device_buffer.clone())],
                    [],
                )
                .unwrap();

                self.batches.push(PieBatch {
                    _buffer: device_buffer,
                    start_key,
                    end_key,
                    base_offset: start_offset,
                    descriptor_set,
                    vbo: None,
                });
            }
        }

        let midi_time = midi_file.current_time().as_seconds_f64();
        let screen_start = (midi_time * midi_file.ticks_per_second() as f64) as i32;
        let screen_end = ((midi_time + view_range) * midi_file.ticks_per_second() as f64) as i32;

        let push_constants = gs::PushConstants {
            start_time: screen_start,
            end_time: screen_end,
            screen_width: img_dims[0] as i32,
            screen_height: img_dims[1] as i32,
        };

        let border_width = crate::utils::calculate_border_width(
            final_image.image().extent()[0] as f32,
            key_view.visible_range.len() as f32,
        ) as i32;

        self.update_vbo_cache(key_view, midi_file, border_width);

        let clears = vec![
            Some(ClearValue::from(bg_color.unwrap_or([0.0; 4]))),
            Some(ClearValue::from(1.0f32)),
        ];

        let framebuffer = self.get_or_create_framebuffer(final_image.clone());

        let mut command_buffer_builder = AutoCommandBufferBuilder::primary(
            self.cb_allocator.clone(),
            self.gfx_queue.queue_family_index(),
            CommandBufferUsage::OneTimeSubmit,
        )
        .unwrap();

        command_buffer_builder
            .begin_render_pass(
                RenderPassBeginInfo {
                    clear_values: clears,
                    ..RenderPassBeginInfo::framebuffer(framebuffer.clone())
                },
                SubpassBeginInfo {
                    contents: SubpassContents::Inline,
                    ..Default::default()
                },
            )
            .unwrap()
            .set_viewport(
                0,
                [viewport.unwrap_or(Viewport {
                    offset: [0.0, 0.0],
                    extent: [img_dims[0] as f32, img_dims[1] as f32],
                    depth_range: 0.0..=1.0,
                })]
                .into_iter()
                .collect(),
            )
            .unwrap()
            .bind_pipeline_graphics(self.pipeline_clear.clone())
            .unwrap()
            .push_constants(self.pipeline_clear.layout().clone(), 0, push_constants)
            .unwrap();

        for batch in &self.batches {
            command_buffer_builder
                .bind_descriptor_sets(
                    PipelineBindPoint::Graphics,
                    self.pipeline_clear.layout().clone(),
                    0,
                    batch.descriptor_set.clone(),
                )
                .unwrap();

            if let Some(vbo) = &batch.vbo {
                unsafe {
                    command_buffer_builder
                        .bind_vertex_buffers(0, vbo.clone())
                        .unwrap()
                        .draw(vbo.len() as u32, 1, 0, 0)
                        .unwrap();
                }
            }
        }

        command_buffer_builder
            .end_render_pass(Default::default())
            .unwrap();
        let command_buffer = command_buffer_builder.build().unwrap();

        // No wait here: the caller chains the rest of the frame onto this future
        let render_future = before
            .then_execute(self.gfx_queue.clone(), command_buffer)
            .unwrap()
            .boxed();

        let flat_blocks = midi_file.flat_blocks();
        let key_colors: Vec<_> = (0..flat_blocks.len())
            .map(|key| flat_blocks.key_color_at(key, screen_start))
            .collect();

        // Notes starting on screen, plus the ones already playing at the bottom edge
        let rendered_notes = flat_blocks.notes_passed_at(screen_end)
            - flat_blocks.notes_passed_at(screen_start)
            + key_colors.iter().filter(|color| color.is_some()).count() as u64;

        (
            RenderResultData {
                notes_rendered: rendered_notes,
                polyphony: None,
                key_colors,
            },
            render_future,
        )
    }
}

mod vs {
    vulkano_shaders::shader! {
        ty: "vertex",
        path: "shaders/pie/pie.vert"
    }
}

mod gs {
    vulkano_shaders::shader! {
        ty: "geometry",
        path: "shaders/pie/pie.geom"
    }
}

mod fs {
    vulkano_shaders::shader! {
        ty: "fragment",
        path: "shaders/pie/pie.frag"
    }
}
