//! The app's installed-font index: the OS's fonts, found with fontdb.
//!
//! Listing every family ([`Source::families`]) reads only what fontdb indexed, so it is
//! quick: there, every face is taken to cover Latin and none to have a weight axis.
//! Looking one family up by name ([`Source::family`]) reads its files and fills both in.

use std::path::PathBuf;
use std::sync::Arc;

use skrifa::MetadataProvider as _;
use skrifa::raw::FontRef;

use super::{Face, Family, Source};

/// The fonts installed on this machine.
pub struct Installed {
    db: fontdb::Database,
}

impl Installed {
    /// Indexes the OS's fonts. This reads every font directory, so do it once, at launch.
    pub fn load() -> Installed {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        Installed { db }
    }

    fn path(info: &fontdb::FaceInfo) -> Option<PathBuf> {
        match &info.source {
            fontdb::Source::File(path) | fontdb::Source::SharedFile(path, _) => Some(path.clone()),
            fontdb::Source::Binary(_) => None,
        }
    }

    fn face(info: &fontdb::FaceInfo, file: PathBuf) -> Face {
        Face {
            file,
            index: info.index,
            weight: info.weight.0,
            weight_axis: None,
            width: info.stretch.to_number(),
            italic: info.style != fontdb::Style::Normal,
            fixed_pitch: info.monospaced,
            latin: true,
        }
    }

    fn name(info: &fontdb::FaceInfo) -> Option<&str> {
        info.families.first().map(|(name, _)| name.as_str())
    }
}

/// What the face's own tables say: its weight axis and whether it maps basic Latin.
fn inspect(face: &mut Face, bytes: &[u8]) {
    let Ok(font) = FontRef::from_index(bytes, face.index) else {
        return;
    };
    face.weight_axis = font
        .axes()
        .iter()
        .find(|axis| axis.tag() == skrifa::Tag::new(b"wght"))
        .map(|axis| (axis.min_value() as u16, axis.max_value() as u16));
    let charmap = font.charmap();
    face.latin = ('a'..='z')
        .chain('A'..='Z')
        .all(|ch| charmap.map(ch).is_some());
}

impl Source for Installed {
    fn family(&self, name: &str) -> Option<Family> {
        let mut found: Option<Family> = None;
        for info in self.db.faces() {
            let Some(own) = Self::name(info) else {
                continue;
            };
            if !own.eq_ignore_ascii_case(name) {
                continue;
            }
            let Some(file) = Self::path(info) else {
                continue;
            };
            let mut face = Self::face(info, file);
            if let Ok(bytes) = std::fs::read(&face.file) {
                inspect(&mut face, &bytes);
            }
            found
                .get_or_insert_with(|| Family {
                    name: own.to_owned(),
                    faces: Vec::new(),
                })
                .faces
                .push(face);
        }
        found
    }

    fn families(&self) -> Vec<Family> {
        let mut families: Vec<Family> = Vec::new();
        for info in self.db.faces() {
            let (Some(name), Some(file)) = (Self::name(info), Self::path(info)) else {
                continue;
            };
            let face = Self::face(info, file);
            match families.iter_mut().find(|f| f.name == name) {
                Some(family) => family.faces.push(face),
                None => families.push(Family {
                    name: name.to_owned(),
                    faces: vec![face],
                }),
            }
        }
        families.sort_by_key(|f| f.name.to_lowercase());
        families
    }

    fn read(&self, face: &Face) -> Option<Arc<[u8]>> {
        std::fs::read(&face.file).ok().map(Arc::from)
    }
}
