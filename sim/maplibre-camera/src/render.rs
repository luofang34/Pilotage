//! One camera's offscreen map: renders the camera's view of a pose and
//! reads the image back as packed RGB.

use std::time::Duration;

use cgmath::{Matrix4, SquareMatrix};
use maplibre::{
    coords::LatLon,
    headless::{
        create_headless_renderer,
        environment::HeadlessEnvironment,
        map::{
            HeadlessMap,
            readback::{FrameReadback, PixelLayout},
        },
    },
    plugin::Plugin,
    render::{
        view_state::ExternalAnchor,
        xr::{EyeTarget, ScenePlacement, XrEye, XrFrame},
    },
    style::Style,
};
use thiserror::Error;

use crate::error::CameraError;
use crate::geometry::{BodyPose, Height, Pointing, world_from_eye};
use crate::rig::CameraSpec;

/// Why one frame of a camera was not produced.
#[derive(Debug, Error)]
pub enum FrameError {
    /// The pose is above the ground, but the terrain under it has not loaded,
    /// so the height of the camera is not known yet.
    #[error("the terrain under the vehicle has not loaded yet")]
    GroundUnknown,
    /// The map could not draw the view.
    #[error("the view could not be drawn: {0}")]
    Draw(#[from] maplibre::headless::map::XrFrameError),
    /// The image could not be read back.
    #[error("the image could not be read back: {0}")]
    Readback(#[from] maplibre::headless::map::readback::ReadbackError),
}

/// A camera, its map, and its image buffers.
pub struct Camera {
    /// The camera's image and lens.
    pub spec: CameraSpec,
    map: HeadlessMap,
    color: wgpu::Texture,
    readback: FrameReadback,
    rgb: Vec<u8>,
}

fn plugins() -> Vec<Box<dyn Plugin<HeadlessEnvironment>>> {
    vec![
        Box::new(maplibre::render::RenderPlugin),
        Box::new(maplibre::background::BackgroundPlugin),
        Box::new(maplibre::vector::VectorPlugin::<
            maplibre::vector::DefaultVectorTransferables,
        >::default()),
        Box::new(maplibre::raster::RasterPlugin::<
            maplibre::raster::DefaultRasterTransferables,
        >::default()),
        Box::new(maplibre::hillshade::HillshadePlugin),
        Box::new(maplibre::terrain::TerrainPlugin::<
            maplibre::terrain::DefaultDemTransferables,
        >::default()),
    ]
}

impl Camera {
    /// Builds the camera's map on its own offscreen renderer. Tiles are
    /// fetched over HTTP and cached in `cache_dir`.
    ///
    /// # Errors
    ///
    /// [`CameraError::Renderer`] or [`CameraError::Map`] when the GPU or the
    /// style cannot make a map.
    pub async fn new(
        spec: CameraSpec,
        mut style: Style,
        cache_dir: &str,
    ) -> Result<Self, CameraError> {
        let camera_id = spec.camera_id;
        let (kernel, renderer) =
            create_headless_renderer(spec.width, spec.height, Some(cache_dir.to_owned()))
                .await
                .map_err(|source| CameraError::Renderer { camera_id, source })?;
        maplibre::headless::map::resolve_tile_json_sources(&mut style, kernel.source_client())
            .await;
        let map = HeadlessMap::new(style, renderer, kernel, plugins())
            .map_err(|source| CameraError::Map { camera_id, source })?;
        let format = map
            .head_texture()
            .map_or(wgpu::TextureFormat::Rgba8Unorm, wgpu::Texture::format);
        let color = map.device().create_texture(&wgpu::TextureDescriptor {
            label: Some("camera colour"),
            size: wgpu::Extent3d {
                width: spec.width,
                height: spec.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let readback = FrameReadback::new(&map, spec.width, spec.height);
        let rgb = vec![0; readback.frame_len(PixelLayout::Rgb8)];
        Ok(Self {
            spec,
            map,
            color,
            readback,
            rgb,
        })
    }

    /// Renders the view of `pose` through `detent`, pointed by `pointing`
    /// when the camera is on a gimbal, and returns its packed RGB pixels.
    /// `timestamp` is the producer's monotonic time, which advances map
    /// animation and tile requests.
    ///
    /// # Errors
    ///
    /// [`FrameError`] when the ground under a height above ground is not
    /// known yet, or the GPU fails to draw or read the view.
    pub fn render_blocking(
        &mut self,
        pose: &BodyPose,
        pointing: Pointing,
        detent: usize,
        timestamp: Duration,
    ) -> Result<&[u8], FrameError> {
        let position = LatLon::new(pose.latitude_deg, pose.longitude_deg);
        let altitude_meters = match pose.height {
            Height::Msl(metres) => metres,
            Height::Agl(metres) => {
                self.map
                    .terrain_elevation_at(position)
                    .ok_or(FrameError::GroundUnknown)?
                    + metres
            }
        };
        let eye = XrEye {
            world_from_eye: world_from_eye(&self.spec, pose, pointing),
            frustum: self.spec.frustum(detent),
            target: EyeTarget {
                color: Some(self.color.create_view(&Default::default())),
                depth: None,
            },
        };
        self.map.run_xr_frame(XrFrame {
            opaque_environment: true,
            timestamp,
            placement: ScenePlacement {
                anchor: ExternalAnchor {
                    position,
                    altitude_meters,
                },
                world_from_scene: Matrix4::identity(),
            },
            eyes: vec![eye],
            request_overscan: 1.2,
            prefetch: None,
        })?;
        self.readback.read_into_blocking(
            &self.map,
            &self.color,
            PixelLayout::Rgb8,
            &mut self.rgb,
        )?;
        Ok(&self.rgb)
    }
}

#[cfg(test)]
mod tests;
