use std::borrow::Cow;

use sdl2;
use sdl2::video::GLProfile;

use glium_sdl2;

use glium::{Program, VertexBuffer, Surface, DrawParameters, Rect};
use glium::{Depth, DepthTest, Blend, BlendingFunction, LinearBlendingFactor};
use glium::index::{PrimitiveType, NoIndices};
use glium::uniforms::{MagnifySamplerFilter, MinifySamplerFilter};
use glium::program::ProgramCreationInput;
use glium::texture::{Texture2d, UncompressedFloatFormat, MipmapsOption};
use glium::texture::{Texture2dDataSource, RawImage2d, ClientFormat};
use glium::texture::{DepthTexture2d, DepthFormat};
use glium::framebuffer::SimpleFrameBuffer;

use super::{TextureDepth, BlendMode, DisplayDepth, SemiTransparencyMode};
use super::{VRAM_WIDTH_PIXELS, VRAM_HEIGHT};

pub struct Renderer {
    /// Glium display
    window: glium_sdl2::SDL2Facade,
    /// Texture used as the target (bound to a framebuffer object) for
    /// the render commands.
    fb_out: Texture2d,
    /// Framebuffer horizontal resolution (native: 1024)
    fb_out_x_res: u16,
    /// Framebuffer vertical resolution (native: 512)
    fb_out_y_res: u16,
    /// Texture used to store the VRAM for texture mapping
    fb_texture: Texture2d,
    /// Depth buffer used for front-to-back drawing
    depth_texture: DepthTexture2d,
    /// Program used to process draw commands
    command_program: Program,
    /// Permanent vertex buffer used to store pending opaque draw commands
    opaque_vertex_buffer: VertexBuffer<CommandVertex>,
    /// Current number or vertices in the opaque vertex buffer
    opaque_vertices: u32,
    /// Permanent vertex buffer used to store pending semi-transparent
    /// draw commands
    semi_transparent_vertex_buffer: VertexBuffer<CommandVertex>,
    /// Current number or vertices in the semi-transparent vertex buffer
    semi_transparent_vertices: u32,
    /// Drawing order of the primitives
    order: u32,
    /// List of queued opaque draw commands. Each command contains a
    /// primitive type (triangle or line) and a number of vertices to
    /// be drawn from the `vertex_buffer`.
    opaque_command_queue: Vec<(PrimitiveType, u32)>,
    /// Current draw command. Will be pushed onto the `command_queue`
    /// if a new command needs to be started.
    current_opaque_command: (PrimitiveType, u32),
    /// List of queued semi-transparent draw commands. Each command
    /// contains a primitive type (triangle or line), a semi-transparency mode
    /// and a number of vertices to be drawn from the `vertex_buffer`.
    semi_transparent_command_queue: Vec<(PrimitiveType,
                                         SemiTransparencyMode,
                                         u32)>,
    /// Current draw command. Will be pushed onto the `command_queue`
    /// if a new command needs to be started.
    current_semi_transparent_command: (PrimitiveType,
                                       SemiTransparencyMode,
                                       u32),
    /// Current draw offset
    offset: (i16, i16),
    /// Parameters for draw commands
    command_params: DrawParameters<'static>,
    /// Program used to display the visible part of the framebuffer
    output_program: Program,
    /// Program used to upload new textures into the framebuffer
    image_load_program: Program,
}

impl Renderer {

    pub fn new(sdl_context: &sdl2::Sdl) -> Renderer {
        use glium_sdl2::DisplayBuild;
        // Size of the framebuffer emulating the Playstation VRAM for
        // draw commands. Can be increased.
        let fb_out_x_res = VRAM_WIDTH_PIXELS as u32;
        let fb_out_y_res = VRAM_HEIGHT as u32;
        // Internal format for the framebuffer. The real console uses
        // RGB 555 + one "mask" bit which we store as alpha.
        let fb_out_format = UncompressedFloatFormat::U5U5U5U1;

        // Video output resolution ("TV screen" size). It's not
        // directly related to the internal framebuffer resolution.
        // Only a game-configured fraction of the framebuffer is
        // displayed at any given moment, several display modes are
        // supported by the console.
        let output_width = 1024;
        let output_height = 768;

        let video_subsystem = sdl_context.video().unwrap();

        let gl_attr = video_subsystem.gl_attr();
        gl_attr.set_context_version(3, 3);
        gl_attr.set_context_profile(GLProfile::Core);

        // XXX Debug context is likely to be slower, we should make
        // that configurable at some point.
        gl_attr.set_context_flags().debug().set();

        let window =
            video_subsystem.window("Rustation", output_width, output_height)
            .position_centered()
            .build_glium()
            .ok().expect("Can't create SDL2 window");

        // Build the program used to render GPU primitives in the
        // framebuffer
        let command_vs_src = include_str!("shaders/command_vertex.glsl");
        let command_fs_src = include_str!("shaders/command_fragment.glsl");

        let command_program =
            Program::new(&window,
                         ProgramCreationInput::SourceCode {
                             vertex_shader: &command_vs_src,
                             tessellation_control_shader: None,
                             tessellation_evaluation_shader: None,
                             geometry_shader: None,
                             fragment_shader: &command_fs_src,
                             transform_feedback_varyings: None,
                             // Don't mess with the color correction
                             outputs_srgb: true,
                             uses_point_size: false,
                         }).unwrap();

        let opaque_vertex_buffer =
            VertexBuffer::empty_persistent(&window,
                                           VERTEX_BUFFER_LEN as usize)
            .unwrap();

        let semi_transparent_vertex_buffer =
            VertexBuffer::empty_persistent(&window,
                                           VERTEX_BUFFER_LEN as usize)
            .unwrap();

        // In order to have the line size scale with the internal
        // resolution upscale we need to compute the upscaling ratio.
        //
        // XXX I only use the y scaling factor since I assume that
        // both dimensions are scaled by the same ratio. Otherwise
        // we'd have to change the line thickness depending on its
        // angle and that would be tricky.
        let scaling_factor = fb_out_y_res as f32 / 512.;

        let command_params = DrawParameters {
            // Default to full screen
            scissor: Some(Rect {
                left: 0,
                bottom: 0,
                width: fb_out_x_res,
                height: fb_out_y_res,
            }),
            line_width: Some(scaling_factor),
            depth: Depth {
                test: DepthTest::IfMore,
                write: true,
                .. Default::default()
            },
            ..Default::default()
        };

        // The framebuffer starts uninitialized
        let default_color = Some((0.5, 0.2, 0.1, 0.0));

        let fb_out = Texture2d::empty_with_format(&window,
                                                  fb_out_format,
                                                  MipmapsOption::NoMipmap,
                                                  fb_out_x_res,
                                                  fb_out_y_res).unwrap();

        fb_out.as_surface().clear(None, default_color, false, None, None);

        let depth_texture =
            DepthTexture2d::empty_with_format(&window,
                                              DepthFormat::F32,
                                              MipmapsOption::NoMipmap,
                                              fb_out_x_res,
                                              fb_out_y_res).unwrap();

        // The texture framebuffer is always at the native resolution
        // since textures can be paletted so no filtering is possible
        // on the raw data.
        let fb_texture =
            Texture2d::empty_with_format(&window,
                                         fb_out_format,
                                         MipmapsOption::NoMipmap,
                                         VRAM_WIDTH_PIXELS as u32,
                                         VRAM_HEIGHT as u32).unwrap();

        fb_texture.as_surface().clear(None, default_color, false, None, None);

        // Build the program used to render the framebuffer onto the output
        let output_vs_src = include_str!("shaders/output_vertex.glsl");
        let output_fs_src = include_str!("shaders/output_fragment.glsl");

        let output_program =
            Program::new(&window,
                         ProgramCreationInput::SourceCode {
                             vertex_shader: &output_vs_src,
                             tessellation_control_shader: None,
                             tessellation_evaluation_shader: None,
                             geometry_shader: None,
                             fragment_shader: &output_fs_src,
                             transform_feedback_varyings: None,
                             outputs_srgb: true,
                             uses_point_size: false,
                         }).unwrap();

        // Build the program used to upload textures to the
        // framebuffer
        let load_vs_src = include_str!("shaders/image_load_vertex.glsl");
        let load_fs_src = include_str!("shaders/image_load_fragment.glsl");

        let image_load_program =
            Program::new(&window,
                         ProgramCreationInput::SourceCode {
                             vertex_shader: &load_vs_src,
                             tessellation_control_shader: None,
                             tessellation_evaluation_shader: None,
                             geometry_shader: None,
                             fragment_shader: &load_fs_src,
                             transform_feedback_varyings: None,
                             outputs_srgb: true,
                             uses_point_size: false,
                         }).unwrap();

        Renderer {
            window: window,
            fb_out: fb_out,
            depth_texture: depth_texture,
            fb_out_x_res: fb_out_x_res as u16,
            fb_out_y_res: fb_out_y_res as u16,
            fb_texture: fb_texture,
            command_program: command_program,
            opaque_vertex_buffer: opaque_vertex_buffer,
            opaque_vertices: 0,
            semi_transparent_vertex_buffer: semi_transparent_vertex_buffer,
            semi_transparent_vertices: 0,
            order: 0,
            opaque_command_queue: Vec::new(),
            current_opaque_command: (PrimitiveType::TrianglesList, 0),
            semi_transparent_command_queue: Vec::new(),
            current_semi_transparent_command: (PrimitiveType::TrianglesList,
                                               SemiTransparencyMode::Average,
                                               0),
            offset: (0, 0),
            command_params: command_params,
            output_program: output_program,
            image_load_program: image_load_program,
        }
    }

    /// Add a triangle to the draw buffer
    pub fn push_triangle(&mut self,
                         mut vertices: [CommandVertex; 3],
                         semi_transparency_mode: SemiTransparencyMode) {

        self.push_primitive(PrimitiveType::TrianglesList,
                            &mut vertices);

        if vertices[0].semi_transparent != 0 {
            self.push_semi_transparent_primitive(PrimitiveType::TrianglesList,
                                                 &vertices,
                                                 semi_transparency_mode);
        }
    }

    /// Add a quad to the draw buffer
    pub fn push_quad(&mut self,
                     vertices: [CommandVertex; 4],
                     semi_transparency_mode: SemiTransparencyMode) {
        self.push_triangle([vertices[0], vertices[1], vertices[2]],
                           semi_transparency_mode);
        self.push_triangle([vertices[1], vertices[2], vertices[3]],
                           semi_transparency_mode);
    }

    /// Add a line to the draw buffer
    pub fn push_line(&mut self,
                     mut vertices: [CommandVertex; 2],
                     semi_transparency_mode: SemiTransparencyMode) {

        self.push_primitive(PrimitiveType::LinesList,
                            &mut vertices);

        if vertices[0].semi_transparent != 0 {
            self.push_semi_transparent_primitive(PrimitiveType::LinesList,
                                                 &vertices,
                                                 semi_transparency_mode);
        }
    }

    /// Add a primitive to the draw buffer to be drawn during the
    /// opaque pass
    fn push_primitive(&mut self,
                      primitive_type: PrimitiveType,
                      vertices: &mut [CommandVertex]) {
        let primitive_vertices = vertices.len() as u32;

        // Make sure we have enough room left to queue the vertex. We
        // need to push two triangles to draw a quad, so 6 vertex
        if self.opaque_vertices + primitive_vertices > VERTEX_BUFFER_LEN {
            // The vertex attribute buffers are full, force an early
            // draw
            self.draw();
        }

        for v in vertices.iter_mut() {
            v.order = self.order;
        }

        self.order += 1;

        let (mut cmd_type, mut cmd_len) = self.current_opaque_command;

        if primitive_type != cmd_type {
            // We have to change the primitive type. Push the current
            // command onto the queue and start a new one.
            if cmd_len > 0 {
                self.opaque_command_queue.push(self.current_opaque_command);
            }

            cmd_type = primitive_type;
            cmd_len = 0;
        }

        // Copy the vertices into the vertex buffer. We fill the
        // buffer from the end going backwards so that we can use the
        // Z-buffer to reduce overdraw.
        let end = VERTEX_BUFFER_LEN - self.opaque_vertices;
        let start = end - primitive_vertices;

        let end = end as usize;
        let start = start as usize;

        let slice = self.opaque_vertex_buffer.slice(start..end).unwrap();
        slice.write(vertices);

        self.opaque_vertices += primitive_vertices;
        self.current_opaque_command = (cmd_type, cmd_len + primitive_vertices);
    }

    /// Add a primitive to the draw buffer to be drawn during the semi-transparent pass
    fn push_semi_transparent_primitive(&mut self,
                                       primitive_type: PrimitiveType,
                                       vertices: &[CommandVertex],
                                       mode: SemiTransparencyMode) {
        let primitive_vertices = vertices.len() as u32;

        // Make sure we have enough room left to queue the vertex. We
        // need to push two triangles to draw a quad, so 6 vertex
        if self.semi_transparent_vertices + primitive_vertices > VERTEX_BUFFER_LEN {
            // The vertex attribute buffers are full, force an early
            // draw
            self.draw();
        }

        // XXX restore me when needed
        // for v in vertices.iter_mut() {
        //     v.order = self.order;
        // }

        //self.order += 1;

        let cur_cmd = self.current_semi_transparent_command;

        let (mut cmd_type, mut cmd_mode, mut cmd_len) = cur_cmd;

        if primitive_type != cmd_type || mode != cmd_mode {
            // We have to change the primitive type or
            // semi-transparency mode. Push the current command onto
            // the queue and start a new one.
            if cmd_len > 0 {
                self.semi_transparent_command_queue.push(cur_cmd);
            }

            cmd_type = primitive_type;
            cmd_mode = mode;
            cmd_len = 0;
        }

        // Copy the vertices into the vertex buffer. We fill the
        // buffer from the end going backwards so that we can use the
        // Z-buffer to reduce overdraw.
        let start = self.semi_transparent_vertices;
        let end = start + primitive_vertices;

        let start = start as usize;
        let end = end as usize;

        let slice = self.semi_transparent_vertex_buffer.slice(start..end).unwrap();
        slice.write(vertices);

        self.semi_transparent_vertices += primitive_vertices;
        self.current_semi_transparent_command = (cmd_type,
                                                 cmd_mode,
                                                 cmd_len + primitive_vertices);
    }

    /// Fill a rectangle in memory with the given color. This method
    /// ignores the mask bit, the drawing area and the drawing offset.
    pub fn fill_rect(&mut self,
                     color: [u8; 3],
                     top: u16, left: u16,
                     bottom: u16, right: u16) {
        // Flush any pending draw commands
        self.draw();

        let top = top as i16;
        let left = left as i16;
        // Fill rect is inclusive
        let bottom = bottom as i16;
        let right = right as i16;

        // Build monochrome command vertex
        let build_vertex = |x: i16, y: i16| -> CommandVertex {
            CommandVertex::new([x, y],
                               color,
                               BlendMode::None,
                               [0; 2],
                               [0; 2],
                               [0; 2],
                               TextureDepth::T4Bpp,
                               false,
                               false)
        };

        let vertices =
            VertexBuffer::new(&self.window,
                              &[build_vertex(left, top),
                                build_vertex(right, top),
                                build_vertex(left, bottom),
                                build_vertex(right, bottom)])
            .unwrap();


        let mut surface = self.fb_out.as_surface();

        let uniforms = uniform! {
            offset: [0, 0],
            fb_texture: &self.fb_texture,
            max_order: 1,
            draw_semi_transparent: 0,
        };

        surface.draw(&vertices,
                     &NoIndices(PrimitiveType::TriangleStrip),
                     &self.command_program,
                     &uniforms,
                     &DrawParameters { ..Default::default() })
            .unwrap();
    }

    /// Set the value of the uniform draw offset
    pub fn set_draw_offset(&mut self, x: i16, y: i16) {
        // Force draw for the primitives with the current offset
        self.draw();

        self.offset = (x, y);
    }

    /// Set the drawing area. Coordinates are offsets in the
    /// PlayStation VRAM
    pub fn set_drawing_area(&mut self,
                            left: u16, top: u16,
                            right: u16, bottom: u16) {
        // Render any pending primitives
        self.draw();

        let (left, top) = self.scale_coords(left, top);
        let (right, bottom) = self.scale_coords(right, bottom);

        if left > right || bottom > top {
            // XXX What should we do here? This happens often because
            // the drawing area is set in two successive calls to set
            // the top_left and then bottom_right so the intermediate
            // value is often wrong.
            self.command_params.scissor = Some(Rect {
                left: 0,
                bottom: 0,
                width: 0,
                height: 0,
            });
        } else {
            // Width and height are inclusive
            let width = right - left + 1;
            let height = top - bottom + 1;

            self.command_params.scissor = Some(Rect {
                left: left,
                bottom: bottom,
                width: width,
                height: height,
            });
        }
    }

    /// Draw the buffered commands and reset the buffers
    pub fn draw(&mut self) {
        let depth_texture =
            DepthTexture2d::empty_with_format(&self.window,
                                              DepthFormat::F32,
                                              MipmapsOption::NoMipmap,
                                              self.fb_out_x_res as u32,
                                              self.fb_out_y_res as u32).unwrap();

        let mut surface =
            SimpleFrameBuffer::with_depth_buffer(&self.window,
                                                 &self.fb_out,
                                                 &depth_texture)
            .unwrap();

        surface.clear_depth(0.0);

        // Push the last pending commands if needed
        let (_, cmd_len) = self.current_opaque_command;

        if cmd_len > 0 {
            self.opaque_command_queue.push(self.current_opaque_command);
        }

        self.draw_opaque(&mut surface);

        // Reset the buffers
        self.opaque_vertices = 0;
        self.opaque_command_queue.clear();
        self.current_opaque_command = (PrimitiveType::TrianglesList, 0);


        // Push the last pending commands if needed
        let cur_cmd = self.current_semi_transparent_command;

        let (_, _ ,cmd_len) = cur_cmd;

        if cmd_len > 0 {
            self.semi_transparent_command_queue.push(cur_cmd);
        }

        self.draw_semi_transparent(&mut surface);

        // Reset the buffers
        self.semi_transparent_vertices = 0;
        self.semi_transparent_command_queue.clear();
        self.current_semi_transparent_command = (PrimitiveType::TrianglesList,
                                                 SemiTransparencyMode::Average,
                                                 0);

        self.order = 0;
    }

    pub fn draw_opaque(&self, surface: &mut SimpleFrameBuffer) {

        if self.opaque_command_queue.is_empty() {
            // Nothing to be done
            return;
        }

        let uniforms = uniform! {
            offset: [self.offset.0 as i32, self.offset.1 as i32],
            fb_texture: &self.fb_texture,
            max_order: self.order as i32,
            draw_semi_transparent: 0,
        };

        let mut vertex_pos = (VERTEX_BUFFER_LEN - self.opaque_vertices) as usize;

        // We iterate in the opposite direction to reduce
        // overdraw. The depth buffer will take care of overlapping
        // geometry.
        for &(cmd_type, cmd_len) in self.opaque_command_queue.iter().rev() {
            let start = vertex_pos;
            let end = start + cmd_len as usize;

            let vertices =
                self.opaque_vertex_buffer.slice(start..end)
                .unwrap();

            surface.draw(vertices,
                         &NoIndices(cmd_type),
                         &self.command_program,
                         &uniforms,
                         &self.command_params).unwrap();

            vertex_pos = end;
        }
    }

    pub fn draw_semi_transparent(&self, surface: &mut SimpleFrameBuffer) {

        if self.semi_transparent_command_queue.is_empty() {
            // Nothing to be done
            return;
        }

        let uniforms = uniform! {
            offset: [self.offset.0 as i32, self.offset.1 as i32],
            fb_texture: &self.fb_texture,
            max_order: self.order as i32,
            draw_semi_transparent: 1,
        };

        let mut vertex_pos = 0;

        let mut params = self.command_params.clone();

        // We iterate in the opposite direction to reduce
        // overdraw. The depth buffer will take care of overlapping
        // geometry.
        for &(cmd_type, cmd_mode, cmd_len) in &self.semi_transparent_command_queue {

            let (blend_fn, _) =
                match cmd_mode {
                    SemiTransparencyMode::Average =>
                        (BlendingFunction::Addition {
                            source: LinearBlendingFactor::ConstantColor,
                            destination: LinearBlendingFactor::ConstantColor,
                        }, 0.5),
                    SemiTransparencyMode::Add =>
                        (BlendingFunction::Addition {
                            source: LinearBlendingFactor::One,
                            destination: LinearBlendingFactor::One,
                        }, 0.5),
                    SemiTransparencyMode::SubstractSource =>
                        (BlendingFunction::ReverseSubtraction {
                            source: LinearBlendingFactor::One,
                            destination: LinearBlendingFactor::One,
                        }, 0.5),
                    SemiTransparencyMode::AddQuarterSource =>
                        (BlendingFunction::ReverseSubtraction {
                            source: LinearBlendingFactor::ConstantAlpha,
                            destination: LinearBlendingFactor::One,
                        }, 0.25),
                };

            params.blend = Blend {
                color: blend_fn,
                alpha: BlendingFunction::AlwaysReplace,
                constant_value: (0.5, 0.5, 0.5, 0.25),
            };

            let start = vertex_pos;
            let end = start + cmd_len as usize;

            let vertices =
                self.semi_transparent_vertex_buffer.slice(start..end)
                .unwrap();

            surface.draw(vertices,
                         &NoIndices(cmd_type),
                         &self.command_program,
                         &uniforms,
                         &params).unwrap();

            vertex_pos = end;
        }
    }

    /// Draw the buffered commands and refresh the video output.
    pub fn display(&mut self,
                   fb_x: u16, fb_y: u16,
                   width: u16, height: u16,
                   depth: DisplayDepth) {
        // Draw any pending commands
        self.draw();

        let params = DrawParameters {
            blend: Blend::alpha_blending(),
            ..Default::default()
        };

        let mut frame = self.window.draw();

        // We sample `fb_out` onto the screen
        let uniforms = uniform! {
            fb: &self.fb_out,
            alpha: 1.0f32,
            depth_24bpp: match depth {
                DisplayDepth::D15Bits => 0,
                DisplayDepth::D24Bits => 1,
            },
        };

        /// Vertex definition for the video output program
        #[derive(Copy, Clone)]
        struct Vertex {
            /// Vertex position on the screen
            position: [f32; 2],
            /// Corresponding coordinate in the framebuffer
            fb_coord: [u16; 2],
        }

        implement_vertex!(Vertex, position, fb_coord);

        let fb_x_start = fb_x;
        let fb_x_end = fb_x + width;
        // OpenGL puts the Y axis in the opposite direction compared
        // to the PlayStation GPU coordinate system so we must start
        // at the bottom here.
        let fb_y_start = fb_y + height;
        let fb_y_end = fb_y;

        // We render a single quad containing the texture to the
        // screen
        let vertices =
            VertexBuffer::new(&self.window,
                              &[Vertex { position: [-1.0, -1.0],
                                         fb_coord: [fb_x_start, fb_y_start] },
                                Vertex { position: [1.0, -1.0],
                                         fb_coord: [fb_x_end, fb_y_start] },
                                Vertex { position: [-1.0, 1.0],
                                         fb_coord: [fb_x_start, fb_y_end] },
                                Vertex { position: [1.0, 1.0],
                                         fb_coord: [fb_x_end, fb_y_end] }])
            .unwrap();

        frame.draw(&vertices,
                   &NoIndices(PrimitiveType::TriangleStrip),
                   &self.output_program,
                   &uniforms,
                   &params).unwrap();

        // Draw the full framebuffer at the bottom right transparently
        // We sample `fb_out` onto the screen
        let vertices =
            VertexBuffer::new(&self.window,
                              &[Vertex { position: [0., -1.0],
                                         fb_coord: [0, 512] },
                                Vertex { position: [1.0, -1.0],
                                         fb_coord: [1024, 512] },
                                Vertex { position: [0.0, -0.5],
                                         fb_coord: [0, 0] },
                                Vertex { position: [1.0, -0.5],
                                         fb_coord: [1024, 0] }])
            .unwrap();

        // Let's use nearest neighbour interpolation for the VRAM
        // dump, doesn't make a lot of sense to interpolate linearly
        // here
        let sampler =
            self.fb_out.sampled()
            .magnify_filter(MagnifySamplerFilter::Nearest)
            .minify_filter(MinifySamplerFilter::Nearest);

        let uniforms = uniform! {
            fb: sampler,
            alpha: 0.7f32,
            depth_24bpp: 0,
        };

        frame.draw(&vertices,
                   &NoIndices(PrimitiveType::TriangleStrip),
                   &self.output_program,
                   &uniforms,
                   &params).unwrap();

        // Flip the buffers and display the new frame
        frame.finish().unwrap();
    }

    /// Convert coordinates in the PlayStation framebuffer to
    /// coordinates in our potentially scaled OpenGL
    /// framebuffer. Coordinates are rounded to the nearest pixel.
    fn scale_coords(&self, x: u16, y: u16) -> (u32, u32) {
        // OpenGL has (0, 0) at the bottom left, the PSX at the top
        // left so we need to complement the y coordinate
        let y = !y & 0x1ff;

        let x = (x as u32 * self.fb_out_x_res as u32 + 512) / 1024;
        let y = (y as u32 * self.fb_out_y_res as u32 + 256) / 512;

        (x, y)
    }

    /// Load an image (texture, palette, ...) into the VRAM
    pub fn load_image(&mut self, load_buffer: LoadBuffer) {
        // XXX must take Mask bit into account. Should also change the
        // alpha mode to conserve the source alpha only (no blending).

        // First we must run any pending command
        self.draw();

        // Target coordinates in VRAM
        let (x, y) = load_buffer.top_left();
        let width = load_buffer.width();
        let height = load_buffer.height();

        let image = load_buffer.into_texture(&self.window);

        let params = DrawParameters {
            ..Default::default()
        };

        /// Vertex definition for the video output program
        #[derive(Copy, Clone)]
        struct Vertex {
            /// Vertex position in VRAM
            position: [u16; 2],
            /// Coordinate in the loaded image
            image_coord: [u16; 2],
        }

        implement_vertex!(Vertex, position, image_coord);

        let x_start = x;
        let x_end = x + width;
        let y_start = y;
        let y_end = y + height;

        // We render a single quad containing the image into the
        // framebuffer
        let vertices =
            VertexBuffer::new(&self.window,
                              &[Vertex { position: [x_start, y_start],
                                         image_coord: [0, 0] },
                                Vertex { position: [x_end, y_start],
                                         image_coord: [width, 0] },
                                Vertex { position: [x_start, y_end],
                                         image_coord: [0, height] },
                                Vertex { position: [x_end, y_end],
                                         image_coord: [width, height] }])
            .unwrap();

        // First we copy the data to the texture VRAM
        let uniforms = uniform! {
            image: &image,
        };

        let mut surface = self.fb_texture.as_surface();

        surface.draw(&vertices,
                     &NoIndices(PrimitiveType::TriangleStrip),
                     &self.image_load_program,
                     &uniforms,
                     &params).unwrap();

        // We'll also write the data to `fb_out` in case the game
        // tries to upload some image directly into the displayed
        // framebuffer.
        let mut surface = self.fb_out.as_surface();

        surface.draw(&vertices,
                     &NoIndices(PrimitiveType::TriangleStrip),
                     &self.image_load_program,
                     &uniforms,
                     &params).unwrap();
    }
}

/// Maximum number of vertex that can be stored in an attribute
/// buffers
const VERTEX_BUFFER_LEN: u32 = 64 * 1024;

/// Vertex definition used by the draw commands
#[derive(Copy,Clone,Debug)]
pub struct CommandVertex {
    /// Position in PlayStation VRAM coordinates
    position: [i16; 2],
    /// Drawing order, later primitives must appear above the earlier
    /// ones.
    order: u32,
    /// RGB color, 8bits per component
    color: [u8; 3],
    /// Texture page (base offset in VRAM used for texture lookup)
    texture_page: [u16; 2],
    /// Texture coordinates within the page
    texture_coord: [u16; 2],
    /// Color Look-Up Table (palette) coordinates in VRAM
    clut: [u16; 2],
    /// Blending mode: 0: no texture, 1: raw-texture, 2: texture-blended
    texture_blend_mode: u8,
    /// Right shift from 16bits: 0 for 16bpp textures, 1 for 8bpp, 2
    /// for 4bpp
    depth_shift: u8,
    /// 1 if dithering is enabled for this primitive, otherwise 0
    dither: u8,
    /// 1 if the primitive is semi-transparent, otherwise 0
    semi_transparent: u8,
}

implement_vertex!(CommandVertex, position, order, color,
                  texture_page, texture_coord, clut, texture_blend_mode,
                  depth_shift, dither, semi_transparent);

impl CommandVertex {
    pub fn new(position: [i16; 2],
               color: [u8; 3],
               blend_mode: BlendMode,
               texture_page: [u16; 2],
               texture_coord: [u16; 2],
               clut: [u16; 2],
               texture_depth: TextureDepth,
               dither: bool,
               semi_transparent: bool) -> CommandVertex {

        let blend_mode =
            match blend_mode {
                BlendMode::None => 0,
                BlendMode::Raw => 1,
                BlendMode::Blended => 2,
            };

        let depth_shift =
            match texture_depth {
                TextureDepth::T4Bpp => 2,
                TextureDepth::T8Bpp => 1,
                TextureDepth::T16Bpp => 0,
            };

        CommandVertex {
            position: position,
            order: 0,
            color: color,
            texture_page: texture_page,
            texture_coord: texture_coord,
            texture_blend_mode: blend_mode,
            clut: clut,
            depth_shift: depth_shift,
            dither: dither as u8,
            semi_transparent: semi_transparent as u8,
        }
    }
}

/// Buffer used to store images while they're loaded into the GPU
/// word-by-word through GP0
pub struct LoadBuffer {
    /// Buffer containing the individual pixels, top-left to
    /// bottom-right
    buf: Vec<u16>,
    /// Width in pixels
    width: u16,
    /// Height
    height: u16,
    /// Coordinate of the top-left corner of target
    /// location in VRAM
    top_left: (u16, u16),
}

impl LoadBuffer {
    pub fn new(x: u16, y: u16, width: u16, height: u16) -> LoadBuffer {
        let size = (width as usize) * (height as usize);

        // Round capacity up to the next even value since we always
        // upload two pixels at a time
        let size = (size + 1) & !1;

        LoadBuffer {
            buf: Vec::with_capacity(size),
            width: width,
            height: height,
            top_left: (x, y),
        }
    }

    /// Build an empty LoadBuffer expecting no data
    pub fn null() -> LoadBuffer {
        LoadBuffer {
            buf: Vec::new(),
            width: 0,
            height: 0,
            top_left: (0, 0),
        }
    }

    pub fn width(&self) -> u16 {
        self.width
    }

    pub fn height(&self) -> u16 {
        self.height
    }

    pub fn top_left(&self) -> (u16, u16) {
        self.top_left
    }

    /// Called a when a new word is received in GP0. Extract the two
    /// pixels and store them in the buffer
    pub fn push_word(&mut self, word: u32) {
        // Unfortunately OpenGL puts the color components the other
        // way around: it uses BGRA 5551 while the PSX uses MRGB 1555
        // so we have to shuffle the components around
        fn shuffle_components(color: u16) -> u16 {
            let alpha = color >> 15;
            let r = (color >> 10) & 0x1f;
            let g = (color >> 5) & 0x1f;
            let b = color & 0x1f;

            alpha | (r << 1) | (g << 6) | (b << 11)
        }

        let p0 = shuffle_components(word as u16);
        let p1 = shuffle_components((word >> 16) as u16);

        self.buf.push(p0);
        self.buf.push(p1);
    }

    pub fn into_texture(self, window: &glium_sdl2::SDL2Facade) -> Texture2d {
        Texture2d::with_format(window,
                               self,
                               UncompressedFloatFormat::U5U5U5U1,
                               MipmapsOption::NoMipmap).unwrap()
    }
}

impl<'a> Texture2dDataSource<'a> for LoadBuffer {
    type Data = u16;

    fn into_raw(self) -> RawImage2d<'a, u16> {
        let width = self.width as u32;
        let height = self.height as u32;

        let mut data = self.buf;

        // We might have one pixel too many because of the padding to
        // 32bits during the upload. Glium will panic if `data` is
        // bigger than expected.
        data.truncate((width * height) as usize);

        RawImage2d {
            data: Cow::Owned(data),
            width: width,
            height: height,
            format: ClientFormat::U5U5U5U1,
        }
    }
}
