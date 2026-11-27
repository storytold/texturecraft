//! Installs the bundled projects in the data folder for File > Open Sample Project.

mod shipped_sample;

use std::io::ErrorKind;
use std::path::Path;

use shipped_sample::ShippedSample;

/// The folder under the data folder that holds the sample projects
pub(crate) const SAMPLES_FOLDER: &str = "Samples";

const SHIPPED_SAMPLES: &[ShippedSample] =
    &[ShippedSample { file_name: "Material Ball.tcraft", bytes: include_bytes!("../../../assets/samples/Material Ball.tcraft") }];

/// Makes sure `folder` exists and holds every shipped sample. A file that is already there is
/// kept as it is, so a sample the user edited and saved stays.
pub(crate) fn install(folder: &Path) -> Result<(), String> {
    std::fs::create_dir_all(folder).map_err(|error| format!("{}: {error}", folder.display()))?;

    for sample in SHIPPED_SAMPLES {
        let destination = folder.join(sample.file_name);
        match destination.symlink_metadata() {
            Ok(_) => continue,
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(format!("{}: {error}", destination.display())),
        }

        sample.install_missing(folder, &destination).map_err(|error| format!("{}: {error}", destination.display()))?;
    }

    Ok(())
}
