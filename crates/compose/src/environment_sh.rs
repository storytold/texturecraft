//! The viewport environment's diffuse light as spherical harmonics, worked out once per environment, and Baked Lighting Environment's shading with it

use std::sync::{Arc, Mutex, Weak};

use texturecraft_core::project::DisplaySettings;
use texturecraft_core::{Channel, DIELECTRIC_REFLECTANCE, finite_unit, mix3};
use texturecraft_geom::glam::Vec3;

use crate::environment_sky::{VIEWPORT_WIDTH, dir_of, environment_linear, radiance, sh_basis, sh9_averaged};
use crate::store::{ImageRes, ImageStore};

/// The diffuse light of the viewport environment and the turn the viewport gives it
#[derive(Clone, Copy)]
pub struct EnvironmentSh {
    coefficients: [[f32; 3]; 9],
    /// The viewport's environment rotation (radians)
    rotation: f32,
}

/// The last coefficients, keyed by a built-in name or the identity of an imported image
type CacheEntry = (String, Option<Weak<ImageRes>>, [[f32; 3]; 9]);

static CACHE: Mutex<Option<CacheEntry>> = Mutex::new(None);

impl EnvironmentSh {
    /// The diffuse light of the environment `display` shows (an imported image from `images`), worked out once per environment
    pub fn of(display: &DisplaySettings, images: &ImageStore) -> EnvironmentSh {
        let image = display.environment_resource().and_then(|name| images.get(name));
        let same = |entry: &CacheEntry| match (&entry.1, &image) {
            (Some(identity), Some(image)) => std::ptr::eq(identity.as_ptr(), Arc::as_ptr(image)),
            (None, None) => entry.0 == display.environment,
            _ => false,
        };
        let rotation = display.env_rotation.to_radians();

        let cached = CACHE.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).as_ref().filter(|entry| same(entry)).map(|entry| entry.2);
        if let Some(coefficients) = cached {
            return EnvironmentSh { coefficients, rotation };
        }

        // Average outside the lock so a Rayon worker can take another tile that needs the cache
        let coefficients = match image {
            Some(image) => image_sh(image),
            None => built_in_sh(&display.environment),
        };
        *CACHE.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some((display.environment.clone(), image.map(Arc::downgrade), coefficients));
        EnvironmentSh { coefficients, rotation }
    }

    /// The diffuse light reaching a surface facing world direction `normal` with the environment turned `horizontal` and tilted `vertical` (radians) past the viewport's turn
    pub fn irradiance(&self, normal: Vec3, horizontal: f32, vertical: f32) -> Vec3 {
        let (sin_tilt, cos_tilt) = vertical.sin_cos();
        let tilted = Vec3::new(normal.x, cos_tilt * normal.y - sin_tilt * normal.z, sin_tilt * normal.y + cos_tilt * normal.z);

        // Turned as the viewport turns its environment
        let (sin_turn, cos_turn) = (self.rotation + horizontal).sin_cos();
        let direction = Vec3::new(cos_turn * tilted.x - sin_turn * tilted.z, tilted.y, sin_turn * tilted.x + cos_turn * tilted.z);
        let light = sh_basis(direction).iter().zip(&self.coefficients).fold(Vec3::ZERO, |sum, (basis, coefficient)| sum + Vec3::from(*coefficient) * *basis);
        light.max(Vec3::ZERO)
    }

    /// Baked Lighting Environment's lit base color: linear `albedo`, its metal read from `channels` (straight values by [`Channel::index`]), in `light` (the environment's diffuse light at the surface, exposure applied), darkened by ambient occlusion `ao` by `[diffuse, specular]` occlusion. The viewer looks straight at the surface, so what it reflects is the light around the normal
    pub fn shade(albedo: [f32; 3], light: Vec3, ao: f32, [diffuse_occlusion, specular_occlusion]: [f32; 2], channels: &[[f32; 3]]) -> [f32; 3] {
        let ao = if ao.is_finite() { ao.clamp(0.0, 1.0) } else { 1.0 };
        let metal = finite_unit(channels.get(Channel::Metallic.index()).map_or(0.0, |m| m[0]));
        let f0 = mix3([DIELECTRIC_REFLECTANCE; 3], albedo, metal);
        let diffuse = 1.0 + (ao - 1.0) * finite_unit(diffuse_occlusion);
        let specular = 1.0 + (ao - 1.0) * finite_unit(specular_occlusion);
        let incoming = light.to_array();
        std::array::from_fn(|k| finite_unit(albedo[k] * (1.0 - metal) * incoming[k] * diffuse + f0[k] * incoming[k] * specular))
    }
}

/// The coefficients of built-in environment `name` as the viewport works them out: drawn at its width, then averaged
fn built_in_sh(name: &str) -> [[f32; 3]; 9] {
    let (width, height) = (VIEWPORT_WIDTH, VIEWPORT_WIDTH / 2);
    sh9_averaged(width, height, |x, y| radiance(name, dir_of((x as f32 + 0.5) / width as f32, (y as f32 + 0.5) / height as f32)))
}

/// The coefficients of an imported equirect image, every texel averaged in as the viewport does
fn image_sh(image: &ImageRes) -> [[f32; 3]; 9] {
    let linear = |v: f32| environment_linear(v, image.srgb);
    sh9_averaged(image.width, image.height, |x, y| {
        let texel = image.rgba.get((y as usize) * image.width as usize + x as usize).copied().unwrap_or([0.0; 4]);
        Vec3::new(linear(texel[0]), linear(texel[1]), linear(texel[2]))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A uniform environment lights every direction alike, whatever the turn; brighter skies light more
    #[test]
    fn a_uniform_image_lights_every_way_alike() {
        for (width, height) in [(1, 1), (32, 16)] {
            let image = ImageRes::new(width, height, vec![[0.5, 0.5, 0.5, 1.0]; (width * height) as usize], false);
            let sh = EnvironmentSh { coefficients: image_sh(&image), rotation: 1.0 };

            for normal in [Vec3::Y, Vec3::X, Vec3::NEG_Y, Vec3::new(0.6, 0.0, 0.8)] {
                let light = sh.irradiance(normal, 0.3, 0.7);
                assert!((light - Vec3::splat(0.5)).abs().max_element() < 0.02, "{width}x{height} {normal}: {light}");
            }
        }
    }

    /// The studio's key light is up and to one side: a surface facing up gets more light than one facing down, and turning the environment moves the light around
    #[test]
    fn a_built_in_lights_by_where_the_surface_faces_and_turns() {
        let display = DisplaySettings { environment: "Studio Soft".into(), ..DisplaySettings::default() };
        let sh = EnvironmentSh::of(&display, &ImageStore::default());
        assert!(sh.irradiance(Vec3::Y, 0.0, 0.0).element_sum() > 2.0 * sh.irradiance(Vec3::NEG_Y, 0.0, 0.0).element_sum());
        let side = Vec3::new(0.7, 0.0, 0.7).normalize();
        let (here, turned) = (sh.irradiance(side, 0.0, 0.0), sh.irradiance(side, std::f32::consts::PI, 0.0));
        assert!((here - turned).abs().max_element() > 0.05, "{here} vs {turned}");
        let night = EnvironmentSh::of(&DisplaySettings { environment: "Night".into(), ..DisplaySettings::default() }, &ImageStore::default());
        assert!(night.irradiance(Vec3::Y, 0.0, 0.0).element_sum() < sh.irradiance(Vec3::Y, 0.0, 0.0).element_sum(), "the cache follows the environment");
    }

    #[test]
    fn replacing_an_imported_environment_releases_its_image() {
        let display = DisplaySettings { environment: "resource:lighting".into(), ..DisplaySettings::default() };

        for color in [[1.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0]] {
            let image = Arc::new(ImageRes::new(32, 16, vec![color; 32 * 16], false));
            let image_identity = Arc::downgrade(&image);
            let images = ImageStore::from([("lighting".into(), image)]);

            let lighting = EnvironmentSh::of(&display, &images);
            drop(images);
            let irradiance = lighting.irradiance(Vec3::Y, 0.0, 0.0);

            assert!(irradiance.distance(Vec3::new(color[0], color[1], color[2])) < 0.02);
            assert!(image_identity.upgrade().is_none(), "lighting should release the decoded image with its store");
        }
    }

    /// Tiles on the thread pool that ask for a new light finish: the light is worked out with the cache unlocked, so a worker that steals another tile while it waits for its share of the averaging never waits for its own lock
    #[test]
    fn tiles_asking_for_a_new_light_on_the_thread_pool_finish() {
        use rayon::prelude::*;

        const WORKERS: usize = 8;
        const TILES: usize = 1024;

        // One tile in so many asks for the light; the rest finish at once, so their workers are free to steal the averaging's work
        const LIT_EVERY: usize = 128;
        const ROUNDS: usize = 20;
        const PATIENCE: std::time::Duration = std::time::Duration::from_secs(60);

        let (done, finished) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let Ok(pool) = rayon::ThreadPoolBuilder::new().num_threads(WORKERS).build() else { return };
            pool.install(|| {
                for round in 0..ROUNDS {
                    // Each round shows another environment, so its first tile misses the cache
                    let environment = if round % 2 == 0 { "Sunset" } else { "Night" };
                    let display = DisplaySettings { environment: environment.into(), ..DisplaySettings::default() };
                    (0..TILES).into_par_iter().filter(|tile| tile % LIT_EVERY == 0).for_each(|_| {
                        EnvironmentSh::of(&display, &ImageStore::default());
                    });
                }
            });
            let _ = done.send(());
        });
        assert!(finished.recv_timeout(PATIENCE).is_ok(), "the tiles deadlocked on the cache");
    }

    /// Diffuse occlusion darkens non-metals by the ambient occlusion; metals take their light from the reflections, which only specular occlusion darkens
    #[test]
    fn occlusion_darkens_the_diffuse_and_the_reflections_apart() {
        let mut channels: Vec<[f32; 3]> = Channel::ALL.iter().map(|c| c.default_value()).collect();
        let light = Vec3::splat(1.0);
        let dielectric = |occlusion| EnvironmentSh::shade([0.5; 3], light, 0.2, occlusion, &channels)[0];
        assert!(dielectric([1.0, 0.0]) < dielectric([0.0, 0.0]) * 0.5);
        channels[Channel::Metallic.index()] = [1.0; 3];
        let metal = |occlusion| EnvironmentSh::shade([0.5; 3], light, 0.2, occlusion, &channels)[0];
        assert_eq!(metal([1.0, 0.0]), metal([0.0, 0.0]));
        assert!(metal([0.0, 1.0]) < metal([0.0, 0.0]) * 0.5);
    }

    /// The coefficients of `radiance` read at every place of a fine grid, the integral the averaged ones must match
    fn fine_sh(width: u32, radiance: impl Fn(Vec3) -> Vec3) -> [[f32; 3]; 9] {
        let height = width / 2;
        crate::environment_sky::sh9(width, height, |x, y| radiance(dir_of((x as f32 + 0.5) / width as f32, (y as f32 + 0.5) / height as f32)))
    }

    /// Sunset's sun is smaller than a texel of a coarse grid: averaging the viewport's drawing keeps its light, as the viewport does
    #[test]
    fn a_small_sun_lights_as_the_viewport_shows_it() {
        let display = DisplaySettings { environment: "Sunset".into(), ..DisplaySettings::default() };

        let baked = EnvironmentSh::of(&display, &ImageStore::default()).coefficients;
        let fine = fine_sh(1024, |direction| radiance("Sunset", direction));

        for (baked, fine) in baked.iter().zip(&fine) {
            for (baked, fine) in baked.iter().zip(fine) {
                assert!((baked - fine).abs() < 0.02, "{baked} vs {fine}");
            }
        }
    }

    /// An imported image's small bright patch counts though it falls between the texels a coarse read would take
    #[test]
    fn a_small_bright_patch_of_an_image_counts() {
        const BRIGHT: f32 = 500.0;
        let (width, height) = (1024u32, 512u32);
        let patch = |x: u32, y: u32| (97..101).contains(&x) && (193..197).contains(&y);
        let rgba = (0..width * height).map(|i| if patch(i % width, i / width) { [BRIGHT, BRIGHT, BRIGHT, 1.0] } else { [0.0, 0.0, 0.0, 1.0] }).collect();
        let image = ImageRes::new(width, height, rgba, false);

        let averaged = image_sh(&image)[0][0];
        let exact = crate::environment_sky::sh9(width, height, |x, y| if patch(x, y) { Vec3::splat(BRIGHT) } else { Vec3::ZERO })[0][0];

        assert!(exact > 0.01 && (averaged - exact).abs() < exact * 0.05, "{averaged} vs {exact}");
    }
}
