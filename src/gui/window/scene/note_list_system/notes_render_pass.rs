use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use vulkano::{
    buffer::{Buffer, BufferCreateInfo, BufferUsage, Subbuffer},
    command_buffer::{
        allocator::StandardCommandBufferAllocator, AutoCommandBufferBuilder, CommandBufferUsage,
        RenderPassBeginInfo, SubpassBeginInfo, SubpassContents,
    },
    descriptor_set::{
        allocator::StandardDescriptorSetAllocator, DescriptorSet, WriteDescriptorSet,
    },
    device::{Device, Queue},
    format::Format,
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

use crate::gui::window::keyboard_layout::KeyboardView;

const NOTE_BUFFER_SIZE: u64 = 4_000_000;

#[repr(C)]
#[derive(Default, Debug, Copy, Clone, Zeroable, Pod, Vertex)]
pub struct NoteVertex {
    #[format(R32G32_SFLOAT)]
    pub start_length: [f32; 2],
    #[format(R32_UINT)]
    pub key_color: u32,
}

struct BufferSet {
    vertex_buffers: Vec<Subbuffer<[NoteVertex]>>,
    index: usize,
    allocator: Arc<StandardMemoryAllocator>,
}

fn get_buffer(allocator: Arc<StandardMemoryAllocator>) -> Subbuffer<[NoteVertex]> {
    Buffer::new_slice(
        allocator.clone(),
        BufferCreateInfo {
            usage: BufferUsage::VERTEX_BUFFER,
            ..Default::default()
        },
        AllocationCreateInfo {
            memory_type_filter: MemoryTypeFilter::HOST_SEQUENTIAL_WRITE,
            ..Default::default()
        },
        NOTE_BUFFER_SIZE,
    )
    .expect("failed to create buffer")
}

impl BufferSet {
    fn new(allocator: Arc<StandardMemoryAllocator>) -> Self {
        let buffer = get_buffer(allocator.clone());
        Self {
            vertex_buffers: vec![buffer],
            index: 0,
            allocator,
        }
    }

    fn reset(&mut self) {
        self.index = 0;
    }

    fn next(&mut self) -> &Subbuffer<[NoteVertex]> {
        if self.index == self.vertex_buffers.len() {
            self.vertex_buffers.push(get_buffer(self.allocator.clone()));
        }

        let index = self.index;
        self.index += 1;

        &self.vertex_buffers[index]
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotePassStatus {
    Finished { remaining: u32 },
    HasMoreNotes,
}

#[repr(C)]
#[derive(Default, Debug, Copy, Clone, Zeroable, Pod)]
pub struct KeyPosition {
    left: f32,
    right: f32,
    _padding: [u8; 8],
}

pub struct NoteRenderPass {
    gfx_queue: Arc<Queue>,
    buffer_set: BufferSet,
    pipeline_clear: Arc<GraphicsPipeline>,
    render_pass_clear: Arc<RenderPass>,
    key_locations: Subbuffer<[[KeyPosition; 256]]>,
    depth_buffer: Arc<ImageView>,
    allocator: Arc<StandardMemoryAllocator>,
    cb_allocator: Arc<StandardCommandBufferAllocator>,
    sd_allocator: Arc<StandardDescriptorSetAllocator>,
}

impl NoteRenderPass {
    pub fn new(device: Arc<Device>, queue: Arc<Queue>, format: Format) -> NoteRenderPass {
        let allocator = Arc::new(StandardMemoryAllocator::new_default(device.clone()));
        let gfx_queue = queue;

        let render_pass_clear = vulkano::ordered_passes_renderpass!(gfx_queue.device().clone(),
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
        let key_locations = Buffer::from_iter(
            allocator.clone(),
            BufferCreateInfo {
                usage: BufferUsage::UNIFORM_BUFFER,
                ..Default::default()
            },
            AllocationCreateInfo {
                memory_type_filter: MemoryTypeFilter::HOST_SEQUENTIAL_WRITE,
                ..Default::default()
            },
            [[Default::default(); 256]],
        )
        .unwrap();

        let vs = vs::load(gfx_queue.device().clone())
            .unwrap()
            .entry_point("main")
            .unwrap();
        let fs = fs::load(gfx_queue.device().clone())
            .unwrap()
            .entry_point("main")
            .unwrap();
        let gs = gs::load(gfx_queue.device().clone())
            .unwrap()
            .entry_point("main")
            .unwrap();

        let vertex_input_state = NoteVertex::per_vertex().definition(&vs).unwrap();
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

        let create_info = GraphicsPipelineCreateInfo {
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
        };

        let pipeline_clear = GraphicsPipeline::new(device.clone(), None, create_info).unwrap();

        NoteRenderPass {
            gfx_queue,
            buffer_set: BufferSet::new(allocator.clone()),
            pipeline_clear,
            render_pass_clear,
            depth_buffer,
            key_locations,
            allocator,
            cb_allocator: StandardCommandBufferAllocator::new(device.clone(), Default::default())
                .into(),
            sd_allocator: StandardDescriptorSetAllocator::new(device.clone(), Default::default())
                .into(),
        }
    }

    pub fn draw(
        &mut self,
        final_image: Arc<ImageView>,
        key_view: &KeyboardView,
        view_range: f32,
        border_width: u32,
        bg_color: Option<[f32; 4]>,
        viewport: Option<Viewport>,
        mut fill_buffer: impl FnMut(&Subbuffer<[NoteVertex]>) -> NotePassStatus,
    ) {
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
        }

        {
            let mut keys = self.key_locations.write().unwrap();
            for (write, key) in keys[0].iter_mut().zip(key_view.iter_all_notes()) {
                *write = KeyPosition {
                    left: key.left,
                    right: key.right,
                    _padding: [0; 8],
                };
            }
        }

        // Collect all draw calls in a single render pass
        let mut draw_calls = Vec::new();
        let mut status = NotePassStatus::HasMoreNotes;
        self.buffer_set.reset();

        while status == NotePassStatus::HasMoreNotes {
            let buffer = self.buffer_set.next();
            status = fill_buffer(buffer);

            let items_to_render = match status {
                NotePassStatus::Finished { remaining } => {
                    assert!(remaining <= buffer.len() as u32);
                    remaining
                }
                NotePassStatus::HasMoreNotes => buffer.len() as u32,
            };

            draw_calls.push((buffer.clone(), items_to_render));
        }

        // Build single command buffer with all draw calls
        let mut command_buffer_builder = AutoCommandBufferBuilder::primary(
            self.cb_allocator.clone(),
            self.gfx_queue.queue_family_index(),
            CommandBufferUsage::OneTimeSubmit,
        )
        .unwrap();

        let clear_color = bg_color.unwrap_or([0.0, 0.0, 0.0, 0.0]);
        let clears = vec![Some(clear_color.into()), Some(1.0f32.into())];

        let pipeline = &self.pipeline_clear;
        let render_pass = &self.render_pass_clear;

        let framebuffer = Framebuffer::new(
            render_pass.clone(),
            FramebufferCreateInfo {
                attachments: vec![final_image.clone(), self.depth_buffer.clone()],
                ..Default::default()
            },
        )
        .unwrap();

        let pipeline_layout = pipeline.layout();

        let desc_layout = pipeline_layout.set_layouts().first().unwrap();
        let write_descriptor_set = WriteDescriptorSet::buffer(0, self.key_locations.clone());
        let set = DescriptorSet::new(
            self.sd_allocator.clone(),
            desc_layout.clone(),
            [write_descriptor_set],
            [],
        )
        .unwrap();

        let subpassbegininfo = SubpassBeginInfo {
            contents: SubpassContents::Inline,
            ..Default::default()
        };

        command_buffer_builder
            .begin_render_pass(
                RenderPassBeginInfo {
                    clear_values: clears,
                    ..RenderPassBeginInfo::framebuffer(framebuffer)
                },
                subpassbegininfo,
            )
            .unwrap();

        let push_constants = gs::PushConstants {
            height_time: view_range,
            win_width: img_dims[0] as f32,
            win_height: img_dims[1] as f32,
            border_width,
        };

        unsafe {
            let viewport_val = if let Some(vp) = viewport.clone() {
                vp
            } else {
                Viewport {
                    offset: [0.0, 0.0],
                    extent: [img_dims[0] as f32, img_dims[1] as f32],
                    depth_range: 0.0..=1.0,
                }
            };

            command_buffer_builder
                .bind_pipeline_graphics(pipeline.clone())
                .unwrap()
                .set_viewport(0, vec![viewport_val].into())
                .unwrap()
                .push_constants(pipeline_layout.clone().clone(), 0, push_constants)
                .unwrap()
                .bind_descriptor_sets(
                    PipelineBindPoint::Graphics,
                    pipeline_layout.clone(),
                    0,
                    set.clone(),
                )
                .unwrap();

            // Submit all draw calls within the same render pass
            for (buffer, items_to_render) in draw_calls {
                command_buffer_builder
                    .bind_vertex_buffers(0, buffer)
                    .unwrap()
                    .draw(items_to_render, 1, 0, 0)
                    .unwrap();
            }
        }

        command_buffer_builder
            .end_render_pass(Default::default())
            .unwrap();
        let command_buffer = command_buffer_builder.build().unwrap();

        // Only wait once, after all draw calls
        let future = Box::new(sync::now(self.gfx_queue.device().clone()));
        let after_main_cb = future
            .then_execute(self.gfx_queue.clone(), command_buffer)
            .unwrap();

        let future = after_main_cb
            .boxed()
            .then_signal_fence_and_flush()
            .expect("Failed to signal fence and flush");

        future.wait(None).unwrap();
    }
}

mod gs {
    vulkano_shaders::shader! {
        ty: "geometry",
        path: "shaders/notes/notes.geom",
    }
}

mod vs {
    vulkano_shaders::shader! {
        ty: "vertex",
        src: "
#version 450
layout(location = 0) in vec2 start_length;
layout(location = 1) in uint key_color;

layout(location = 0) out vec2 v_start_length;
layout(location = 1) out uint v_key_color;

void main() {
    v_start_length = start_length;
    v_key_color = key_color;
}"
    }
}

mod fs {
    vulkano_shaders::shader! {
        ty: "fragment",
        path: "shaders/notes/notes.frag"
    }
}
