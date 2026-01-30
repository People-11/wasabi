use std::sync::Arc;

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
    device::Queue,
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
    gui::{
        window::keyboard_layout::KeyboardView,
        GuiRenderer,
    },
    midi::{CakeMIDIFile, CakeSignature},
};

use super::RenderResultData;

#[derive(Default, Debug, Copy, Clone, Zeroable, Pod, Vertex)]
#[repr(C)]
struct CakeNoteColumn {
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

struct CakeBatch {
    buffer: Subbuffer<[i32]>,
    start_key: usize,
    end_key: usize,
    base_offset: usize,
}



pub struct CakeRenderer {
    gfx_queue: Arc<Queue>,
    batches: Vec<CakeBatch>,
    pipeline_clear: Arc<GraphicsPipeline>,
    render_pass_clear: Arc<RenderPass>,
    allocator: Arc<StandardMemoryAllocator>,
    depth_buffer: Arc<ImageView>,
    cb_allocator: Arc<StandardCommandBufferAllocator>,
    sd_allocator: Arc<StandardDescriptorSetAllocator>,
    current_file_signature: Option<CakeSignature>,
}

impl CakeRenderer {
    pub fn new(renderer: &GuiRenderer) -> CakeRenderer {
        let allocator = Arc::new(StandardMemoryAllocator::new_default(
            renderer.device.clone(),
        ));

        let gfx_queue = renderer.queue.clone();

        let render_pass_clear = vulkano::ordered_passes_renderpass!(gfx_queue.device().clone(),
            attachments: {
                final_color: {
                    format: renderer.format,
                    samples: 1,
                    load_op: Clear,
                    store_op: Store,
                },
                depth: {
                    format: Format::D16_UNORM,
                    samples: 1,
                    load_op: Clear,
                    store_op: Store,
                }
            },
            passes: [
                {
                    color: [final_color],
                    depth_stencil: {depth},
                    input: []
                }
            ]
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

        let vs = vs::load(gfx_queue.device().clone())
            .expect("failed to create shader module")
            .entry_point("main")
            .unwrap();
        let fs = fs::load(gfx_queue.device().clone())
            .expect("failed to create shader module")
            .entry_point("main")
            .unwrap();
        let gs = gs::load(gfx_queue.device().clone())
            .expect("failed to create shader module")
            .entry_point("main")
            .unwrap();

        let vertex_input_state = CakeNoteColumn::per_vertex().definition(&vs).unwrap();
        let stages = [
            PipelineShaderStageCreateInfo::new(vs),
            PipelineShaderStageCreateInfo::new(fs),
            PipelineShaderStageCreateInfo::new(gs),
        ];
        let layout = PipelineLayout::new(
            renderer.device.clone(),
            PipelineDescriptorSetLayoutCreateInfo::from_stages(&stages)
                .into_pipeline_layout_create_info(renderer.device.clone())
                .unwrap(),
        )
        .unwrap();
        let subpass = Subpass::from(render_pass_clear.clone(), 0).unwrap();

        let pipeline_clear = GraphicsPipeline::new(
            renderer.device.clone(),
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



        CakeRenderer {
            gfx_queue,
            batches: vec![],
            pipeline_clear,
            render_pass_clear,
            depth_buffer,
            allocator,
            cb_allocator: StandardCommandBufferAllocator::new(
                renderer.device.clone(),
                StandardCommandBufferAllocatorCreateInfo::default(),
            )
            .into(),
            sd_allocator: StandardDescriptorSetAllocator::new(
                renderer.device.clone(),
                StandardDescriptorSetAllocatorCreateInfo::default(),
            )
            .into(),
            current_file_signature: None,
        }
    }

    pub fn draw(
        &mut self,
        key_view: &KeyboardView,
        final_image: Arc<ImageView>,
        midi_file: &mut CakeMIDIFile,
        view_range: f64,
    ) -> RenderResultData {
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

        let curr_signature = midi_file.cake_signature();
        if self.current_file_signature.as_ref() != Some(&curr_signature) {
            self.current_file_signature = Some(curr_signature);
            self.batches.clear();
            
            let flat_blocks = midi_file.flat_blocks();
            let mut current_batch_start = 0;
            let mut current_batch_size = 0;
            let mut current_start_offset = 0;
            let target_batch_size = 128 * 1024 * 1024; // 128MB

            for i in 0..flat_blocks.len() {
                let info = flat_blocks.get_block_info(i);
                // 4 bytes per int
                let size_bytes = info.tree_len * 4;

                if current_batch_size + size_bytes > target_batch_size && current_batch_size > 0 {
                    // Flush batch
                    let end_offset = info.tree_offset;
                    let slice = &flat_blocks.tree_buffer[current_start_offset..end_offset];
                    let buffer = Buffer::from_iter(
                        self.allocator.clone(),
                        BufferCreateInfo {
                            usage: BufferUsage::STORAGE_BUFFER,
                            ..Default::default()
                        },
                        AllocationCreateInfo {
                            memory_type_filter: MemoryTypeFilter::HOST_SEQUENTIAL_WRITE,
                            ..Default::default()
                        },
                        slice.iter().copied(),
                    ).unwrap();

                    self.batches.push(CakeBatch {
                        buffer,
                        start_key: current_batch_start,
                        end_key: i,
                        base_offset: current_start_offset,
                    });

                    current_batch_start = i;
                    current_batch_size = 0;
                    current_start_offset = end_offset;
                }

                current_batch_size += size_bytes;
            }

            // Flush final batch
            if current_batch_start < flat_blocks.len() {
                let slice = &flat_blocks.tree_buffer[current_start_offset..];
                let buffer = Buffer::from_iter(
                    self.allocator.clone(),
                    BufferCreateInfo {
                        usage: BufferUsage::STORAGE_BUFFER,
                        ..Default::default()
                    },
                    AllocationCreateInfo {
                        memory_type_filter: MemoryTypeFilter::HOST_SEQUENTIAL_WRITE,
                        ..Default::default()
                    },
                    slice.iter().copied(),
                ).unwrap();

                self.batches.push(CakeBatch {
                    buffer,
                    start_key: current_batch_start,
                    end_key: flat_blocks.len(),
                    base_offset: current_start_offset,
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

        let (clears, pipeline, render_pass) = (
            vec![
                Some(ClearValue::from([0.0f32, 0.0, 0.0, 0.0])),
                Some(ClearValue::from(1.0f32)),
            ],
            &self.pipeline_clear,
            &self.render_pass_clear,
        );

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
                [Viewport {
                    offset: [0.0, 0.0],
                    extent: [img_dims[0] as f32, img_dims[1] as f32],
                    depth_range: 0.0..=1.0,
                }]
                .into_iter()
                .collect(),
            )
            .unwrap()
            .bind_pipeline_graphics(self.pipeline_clear.clone())
            .unwrap()
            .push_constants(self.pipeline_clear.layout().clone(), 0, push_constants)
            .unwrap();

        let flat_blocks = midi_file.flat_blocks();

        for batch in &self.batches {
            let data_descriptor = DescriptorSet::new(
                self.sd_allocator.clone(),
                desc_layout.clone(),
                [WriteDescriptorSet::buffer(0, batch.buffer.clone())],
                [],
            )
            .unwrap();

            command_buffer_builder.bind_descriptor_sets(
                PipelineBindPoint::Graphics,
                self.pipeline_clear.layout().clone(),
                0,
                data_descriptor,
            ).unwrap();

            let mut batch_instances = Vec::new();
            
            // Black keys
            for i in batch.start_key..batch.end_key {
                let key = key_view.note(i);
                if key.black {
                    let info = flat_blocks.get_block_info(i);
                    batch_instances.push(CakeNoteColumn {
                        tree_offset: (info.tree_offset - batch.base_offset) as i32,
                        border_width,
                        start: info.start_time as i32,
                        end: info.end_time as i32,
                        left: key.left,
                        right: key.right,
                    });
                }
            }
            // White keys
            for i in batch.start_key..batch.end_key {
                let key = key_view.note(i);
                if !key.black {
                    let info = flat_blocks.get_block_info(i);
                    batch_instances.push(CakeNoteColumn {
                        tree_offset: (info.tree_offset - batch.base_offset) as i32,
                        border_width,
                        start: info.start_time as i32,
                        end: info.end_time as i32,
                        left: key.left,
                        right: key.right,
                    });
                }
            }

            if !batch_instances.is_empty() {
                let instance_buffer = Buffer::from_iter(
                    self.allocator.clone(),
                    BufferCreateInfo {
                        usage: BufferUsage::VERTEX_BUFFER,
                        ..Default::default()
                    },
                    AllocationCreateInfo {
                        memory_type_filter: MemoryTypeFilter::HOST_SEQUENTIAL_WRITE,
                        ..Default::default()
                    },
                    batch_instances,
                )
                .unwrap();

                unsafe {
                    command_buffer_builder
                        .bind_vertex_buffers(0, instance_buffer.clone())
                        .unwrap()
                        .draw(instance_buffer.len() as u32, 1, 0, 0)
                        .unwrap();
                }
            }
        }

        command_buffer_builder
            .end_render_pass(Default::default())
            .unwrap();
        let command_buffer = command_buffer_builder.build().unwrap();

        let now = sync::now(self.gfx_queue.device().clone()).boxed();
        let render_future = now
            .then_execute(self.gfx_queue.clone(), command_buffer)
            .unwrap()
            .boxed();

        // Calculate the metadata before awaiting the future
        // to keep this more efficient
        let flat_blocks = midi_file.flat_blocks();
        let colors = (0..flat_blocks.len())
            .map(|key| {
                flat_blocks
                    .get_note_at(key, screen_start)
                    .map(|n| n.color)
            })
            .collect();
        let rendered_notes = (0..flat_blocks.len())
            .map(|key| {
                let passed = flat_blocks.get_notes_passed_at(key, screen_end)
                    - flat_blocks.get_notes_passed_at(key, screen_start);

                if flat_blocks.get_note_at(key, screen_start).is_some() {
                    passed as u64 + 1
                } else {
                    passed as u64
                }
            })
            .sum();

        render_future
            .then_signal_fence_and_flush()
            .unwrap()
            .wait(None)
            .unwrap();

        RenderResultData {
            notes_rendered: rendered_notes,
            polyphony: None,
            key_colors: colors,
        }
    }
}

mod vs {
    vulkano_shaders::shader! {
        ty: "vertex",
        path: "shaders/cake/cake.vert"
    }
}

mod gs {
    vulkano_shaders::shader! {
        ty: "geometry",
        path: "shaders/cake/cake.geom"
    }
}

mod fs {
    vulkano_shaders::shader! {
        ty: "fragment",
        path: "shaders/cake/cake.frag"
    }
}
