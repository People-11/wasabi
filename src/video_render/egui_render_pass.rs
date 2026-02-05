use std::sync::Arc;
use bytemuck::{Pod, Zeroable};
use egui::{Context, FullOutput, RawInput, epaint::Primitive};
use vulkano::{
    buffer::{Buffer, BufferCreateInfo, BufferUsage},
    command_buffer::{
        AutoCommandBufferBuilder, PrimaryAutoCommandBuffer,
        RenderPassBeginInfo, SubpassBeginInfo, SubpassContents,
    },
    descriptor_set::{allocator::StandardDescriptorSetAllocator, DescriptorSet, WriteDescriptorSet},
    device::{Device, Queue},
    format::Format,
    image::{
        view::ImageView, Image, ImageCreateInfo, ImageUsage,
        sampler::{Sampler, SamplerCreateInfo, Filter, SamplerAddressMode},
    },
    memory::allocator::{AllocationCreateInfo, MemoryTypeFilter, StandardMemoryAllocator},
    pipeline::{
        graphics::{
            color_blend::{ColorBlendAttachmentState, ColorBlendState, BlendFactor, BlendOp, AttachmentBlend},
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
};

#[repr(C)]
#[derive(Default, Debug, Copy, Clone, Zeroable, Pod, Vertex)]
pub struct EguiVertex {
    #[format(R32G32_SFLOAT)]
    pub pos: [f32; 2],
    #[format(R32G32_SFLOAT)]
    pub uv: [f32; 2],
    #[format(R32_UINT)]
    pub color: u32,
}

/// A unified offscreen egui renderer combining Context management and Vulkan rendering.
pub struct EguiRenderer {
    context: Context,
    raw_input: RawInput,
    render_pass: Arc<RenderPass>,
    pipeline: Arc<GraphicsPipeline>,
    font_texture: Option<Arc<ImageView>>,
    sampler: Arc<Sampler>,
    allocator: Arc<StandardMemoryAllocator>,
    sd_allocator: Arc<StandardDescriptorSetAllocator>,
    white_texture: Arc<ImageView>,
    width: u32, height: u32, ppp: f32, // Stored to re-apply every frame after take()
}

impl EguiRenderer {
    pub fn new(device: Arc<Device>, _queue: Arc<Queue>, format: Format, width: u32, height: u32, ppp: f32) -> Self {
        let allocator = Arc::new(StandardMemoryAllocator::new_default(device.clone()));
        let sd_allocator = Arc::new(StandardDescriptorSetAllocator::new(device.clone(), Default::default()));
        
        // 1. Setup Egui Context
        let context = Context::default();
        let mut fonts = egui::FontDefinitions::default();
        fonts.font_data.insert("poppins".into(), egui::FontData::from_static(include_bytes!("../../assets/Poppins-Medium.ttf")).into());
        fonts.families.get_mut(&egui::FontFamily::Proportional).unwrap().insert(0, "poppins".into());
        fonts.font_data.insert("ubuntu".into(), egui::FontData::from_static(include_bytes!("../../assets/UbuntuSansMono-Medium.ttf")).into());
        fonts.families.get_mut(&egui::FontFamily::Monospace).unwrap().insert(0, "ubuntu".into());
        context.set_fonts(fonts);
        
        let mut style = (*context.style()).clone();
        style.animation_time = 0.0;
        style.visuals.panel_fill = egui::Color32::from_rgb(18, 18, 18);
        style.visuals.window_fill = style.visuals.panel_fill;
        style.visuals.override_text_color = Some(egui::Color32::from_rgb(210, 210, 210));
        context.set_style(style);
        context.set_pixels_per_point(ppp);

        let mut raw_input = RawInput::default();
        raw_input.screen_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::Vec2::new(width as f32 / ppp, height as f32 / ppp)));

        // 2. Setup Vulkan Pipeline
        let render_pass = vulkano::ordered_passes_renderpass!(device.clone(),
            attachments: { color: { format: format, samples: 1, load_op: Load, store_op: Store },
                           depth: { format: Format::D16_UNORM, samples: 1, load_op: Load, store_op: Store } },
            passes: [{ color: [color], depth_stencil: {depth}, input: [] }]
        ).unwrap();

        let vs = vs::load(device.clone()).unwrap().entry_point("main").unwrap();
        let fs = fs::load(device.clone()).unwrap().entry_point("main").unwrap();
        let stages = [PipelineShaderStageCreateInfo::new(vs.clone()), PipelineShaderStageCreateInfo::new(fs)];
        let layout = PipelineLayout::new(device.clone(), PipelineDescriptorSetLayoutCreateInfo::from_stages(&stages)
            .into_pipeline_layout_create_info(device.clone()).unwrap()).unwrap();

        let blend = AttachmentBlend {
            src_color_blend_factor: BlendFactor::One, dst_color_blend_factor: BlendFactor::OneMinusSrcAlpha, color_blend_op: BlendOp::Add,
            src_alpha_blend_factor: BlendFactor::OneMinusDstAlpha, dst_alpha_blend_factor: BlendFactor::One, alpha_blend_op: BlendOp::Add,
        };

        let pipeline = GraphicsPipeline::new(device.clone(), None, GraphicsPipelineCreateInfo {
            stages: stages.into_iter().collect(),
            vertex_input_state: Some(EguiVertex::per_vertex().definition(&vs).unwrap()),
            input_assembly_state: Some(InputAssemblyState { topology: PrimitiveTopology::TriangleList, ..Default::default() }),
            viewport_state: Some(Default::default()),
            rasterization_state: Some(RasterizationState::default()),
            multisample_state: Some(MultisampleState::default()),
            depth_stencil_state: Some(vulkano::pipeline::graphics::depth_stencil::DepthStencilState::default()),
            color_blend_state: Some(ColorBlendState::with_attachment_states(Subpass::from(render_pass.clone(), 0).unwrap().num_color_attachments(),
                ColorBlendAttachmentState { blend: Some(blend), ..Default::default() })),
            dynamic_state: [DynamicState::Viewport, DynamicState::Scissor].into_iter().collect(),
            subpass: Some(Subpass::from(render_pass.clone(), 0).unwrap().into()),
            ..GraphicsPipelineCreateInfo::layout(layout)
        }).unwrap();

        let sampler = Sampler::new(device, SamplerCreateInfo {
            mag_filter: Filter::Linear, min_filter: Filter::Linear, address_mode: [SamplerAddressMode::ClampToEdge; 3], ..Default::default()
        }).unwrap();

        let white_texture = ImageView::new_default(Image::new(allocator.clone(), ImageCreateInfo {
            format: Format::R8G8B8A8_UNORM, extent: [1, 1, 1], usage: ImageUsage::TRANSFER_DST | ImageUsage::SAMPLED, ..Default::default()
        }, Default::default()).unwrap()).unwrap();

        Self { context, raw_input, render_pass, pipeline, font_texture: None, sampler, allocator, sd_allocator, white_texture, width, height, ppp }
    }

    pub fn context(&self) -> &Context { &self.context }

    pub fn begin_frame(&mut self, time: f64) {
        self.raw_input.time = Some(time);
        self.raw_input.screen_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::Vec2::new(self.width as f32 / self.ppp, self.height as f32 / self.ppp)));
        self.context.begin_pass(self.raw_input.take());
    }

    pub fn end_frame(&mut self, cmd_builder: &mut AutoCommandBufferBuilder<PrimaryAutoCommandBuffer>, target: Arc<ImageView>, depth: Arc<ImageView>) -> FullOutput {
        let output = self.context.end_pass();
        let ppp = self.ppp;
        
        // Update Textures
        for (id, delta) in &output.textures_delta.set {
            if let egui::TextureId::Managed(0) = id {
                let [w, h] = delta.image.size();
                let pixels: Vec<u8> = match &delta.image {
                    egui::ImageData::Font(f) => f.pixels.iter().flat_map(|&a| { let v = (a * 255.0) as u8; [v, v, v, v] }).collect(),
                    egui::ImageData::Color(c) => c.pixels.iter().flat_map(|p| p.to_array()).collect(),
                };
                let buf = Buffer::from_iter(self.allocator.clone(), BufferCreateInfo { usage: BufferUsage::TRANSFER_SRC, ..Default::default() },
                    AllocationCreateInfo { memory_type_filter: MemoryTypeFilter::HOST_SEQUENTIAL_WRITE, ..Default::default() }, pixels).unwrap();
                let img = Image::new(self.allocator.clone(), ImageCreateInfo { format: Format::R8G8B8A8_UNORM, extent: [w as u32, h as u32, 1],
                    usage: ImageUsage::TRANSFER_DST | ImageUsage::SAMPLED, ..Default::default() }, Default::default()).unwrap();
                cmd_builder.copy_buffer_to_image(vulkano::command_buffer::CopyBufferToImageInfo::buffer_image(buf, img.clone())).unwrap();
                self.font_texture = Some(ImageView::new_default(img).unwrap());
            }
        }

        let primitives = self.context.tessellate(output.shapes.clone(), output.pixels_per_point);
        let extent = target.image().extent();
        let (w, h) = (extent[0] as f32, extent[1] as f32);

        cmd_builder.begin_render_pass(RenderPassBeginInfo { clear_values: vec![None, None],
            ..RenderPassBeginInfo::framebuffer(Framebuffer::new(self.render_pass.clone(), FramebufferCreateInfo { attachments: vec![target, depth], ..Default::default() }).unwrap())
        }, SubpassBeginInfo { contents: SubpassContents::Inline, ..Default::default() }).unwrap();

        cmd_builder.bind_pipeline_graphics(self.pipeline.clone()).unwrap();
        cmd_builder.push_constants(self.pipeline.layout().clone(), 0, vs::PushConstants { screen_size: [w / ppp, h / ppp] }).unwrap();

        for clipped in primitives {
            if let Primitive::Mesh(mesh) = clipped.primitive {
                if mesh.vertices.is_empty() { continue; }
                let idx_count = mesh.indices.len() as u32;
                let v_buf = Buffer::from_iter(self.allocator.clone(), BufferCreateInfo { usage: BufferUsage::VERTEX_BUFFER, ..Default::default() },
                    AllocationCreateInfo { memory_type_filter: MemoryTypeFilter::HOST_SEQUENTIAL_WRITE, ..Default::default() },
                    bytemuck::cast_slice::<egui::epaint::Vertex, EguiVertex>(&mesh.vertices).to_vec()).unwrap();
                let i_buf = Buffer::from_iter(self.allocator.clone(), BufferCreateInfo { usage: BufferUsage::INDEX_BUFFER, ..Default::default() },
                    AllocationCreateInfo { memory_type_filter: MemoryTypeFilter::HOST_SEQUENTIAL_WRITE, ..Default::default() }, mesh.indices).unwrap();
                
                let tex = if let egui::TextureId::Managed(0) = mesh.texture_id { self.font_texture.clone().unwrap_or(self.white_texture.clone()) } else { self.white_texture.clone() };
                let set = DescriptorSet::new(self.sd_allocator.clone(), self.pipeline.layout().set_layouts()[0].clone(),
                    [WriteDescriptorSet::image_view_sampler(0, tex, self.sampler.clone())], []).unwrap();
                
                let scissor = vulkano::pipeline::graphics::viewport::Scissor {
                    offset: [(clipped.clip_rect.min.x * ppp).round().max(0.0) as u32, (clipped.clip_rect.min.y * ppp).round().max(0.0) as u32],
                    extent: [((clipped.clip_rect.max.x - clipped.clip_rect.min.x) * ppp).round().max(0.0) as u32, ((clipped.clip_rect.max.y - clipped.clip_rect.min.y) * ppp).round().max(0.0) as u32],
                };

                unsafe {
                    cmd_builder.set_viewport(0, [Viewport { offset: [0.0, 0.0], extent: [w, h], depth_range: 0.0..=1.0 }].into_iter().collect()).unwrap()
                        .set_scissor(0, [scissor].into_iter().collect()).unwrap()
                        .bind_descriptor_sets(PipelineBindPoint::Graphics, self.pipeline.layout().clone(), 0, set).unwrap()
                        .bind_vertex_buffers(0, v_buf).unwrap().bind_index_buffer(i_buf).unwrap()
                        .draw_indexed(idx_count, 1, 0, 0, 0).unwrap();
                }
            }
        }
        cmd_builder.end_render_pass(Default::default()).unwrap();
        output
    }
}

mod vs { vulkano_shaders::shader! { ty: "vertex", src: "
    #version 450
    layout(location = 0) in vec2 pos; layout(location = 1) in vec2 uv; layout(location = 2) in uint color;
    layout(location = 0) out vec2 v_uv; layout(location = 1) out vec4 v_color;
    layout(push_constant) uniform PushConstants { vec2 screen_size; } pc;
    vec3 s_to_l(vec3 s) { bvec3 c = lessThan(s, vec3(0.04045)); return mix(pow((s + 0.055) / 1.055, vec3(2.4)), s / 12.92, c); }
    void main() { v_uv = uv; vec4 s = unpackUnorm4x8(color); v_color = vec4(s_to_l(s.rgb), s.a);
        gl_Position = vec4((pos.x / pc.screen_size.x) * 2.0 - 1.0, (pos.y / pc.screen_size.y) * 2.0 - 1.0, 0.0, 1.0); }
" } }
mod fs { vulkano_shaders::shader! { ty: "fragment", src: "
    #version 450
    layout(location = 0) in vec2 v_uv; layout(location = 1) in vec4 v_color;
    layout(location = 0) out vec4 f_color;
    layout(set = 0, binding = 0) uniform sampler2D tex;
    void main() { f_color = v_color * texture(tex, v_uv); }
" } }
