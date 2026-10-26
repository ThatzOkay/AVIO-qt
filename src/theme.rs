use material_colors::{color::Rgb, hct::Hct, scheme::{variant::SchemeContent, Scheme}};
use qtbridge::qobject;

// Same seed avio ships with; the scheme is built the way avio's useM3Theme.ts does (SchemeContent, no extra contrast)
pub const DEFAULT_SEED: &str = "#1bf36e";

pub struct Theme {
    dark: bool,
    seed: String,
    scheme: Scheme,
}

fn build_scheme(seed: &str, dark: bool) -> Option<Scheme> {
    let rgb: Rgb = seed.parse().ok()?;
    Some(SchemeContent::new(Hct::new(rgb), dark, None).scheme.into())
}

impl Default for Theme {
    fn default() -> Self {
        let saved = crate::store::settings_store::snapshot();
        // A hand-edited settings file may hold a bad seed
        let (seed, scheme) = match build_scheme(&saved.seed, saved.dark) {
            Some(scheme) => (saved.seed, scheme),
            None => (
                DEFAULT_SEED.to_string(),
                build_scheme(DEFAULT_SEED, saved.dark).expect("default seed is valid"),
            ),
        };
        Self {
            dark: saved.dark,
            seed,
            scheme,
        }
    }
}

#[qobject(Singleton)]
impl Theme {
    qproperty!("dark", Read = dark, Write = set_dark, Notify = changed);
    qproperty!("seed", Read = seed, Write = set_seed, Notify = changed);
    qproperty!("primary", Read = primary, Notify = changed);
    qproperty!("onPrimary", Read = on_primary, Notify = changed);
    qproperty!("primaryContainer", Read = primary_container, Notify = changed);
    qproperty!("onPrimaryContainer", Read = on_primary_container, Notify = changed);
    qproperty!("secondary", Read = secondary, Notify = changed);
    qproperty!("onSecondary", Read = on_secondary, Notify = changed);
    qproperty!("secondaryContainer", Read = secondary_container, Notify = changed);
    qproperty!("onSecondaryContainer", Read = on_secondary_container, Notify = changed);
    qproperty!("tertiary", Read = tertiary, Notify = changed);
    qproperty!("tertiaryContainer", Read = tertiary_container, Notify = changed);
    qproperty!("onTertiaryContainer", Read = on_tertiary_container, Notify = changed);
    qproperty!("error", Read = error, Notify = changed);
    qproperty!("errorContainer", Read = error_container, Notify = changed);
    qproperty!("surface", Read = surface, Notify = changed);
    qproperty!("onSurface", Read = on_surface, Notify = changed);
    qproperty!("surfaceVariant", Read = surface_variant, Notify = changed);
    qproperty!("onSurfaceVariant", Read = on_surface_variant, Notify = changed);
    qproperty!("surfaceContainerLow", Read = surface_container_low, Notify = changed);
    qproperty!("surfaceContainer", Read = surface_container, Notify = changed);
    qproperty!("surfaceContainerHigh", Read = surface_container_high, Notify = changed);
    qproperty!("surfaceContainerHighest", Read = surface_container_highest, Notify = changed);
    qproperty!("outline", Read = outline, Notify = changed);
    qproperty!("outlineVariant", Read = outline_variant, Notify = changed);
    qproperty!("inverseSurface", Read = inverse_surface, Notify = changed);
    qproperty!("inverseOnSurface", Read = inverse_on_surface, Notify = changed);
    qproperty!("inversePrimary", Read = inverse_primary, Notify = changed);

    // One signal for everything: changing dark or the seed regenerates the whole palette
    #[qsignal]
    fn changed(&mut self);

    pub fn dark(&self) -> bool {
        self.dark
    }

    pub fn set_dark(&mut self, dark: bool) {
        self.dark = dark;
        self.rebuild();
        crate::store::settings_store::update(|s| s.dark = dark);
    }

    pub fn seed(&self) -> String {
        self.seed.clone()
    }

    // Ignores seeds that aren't valid hex colors so a half-typed value doesn't wipe the palette
    pub fn set_seed(&mut self, seed: String) {
        if build_scheme(&seed, self.dark).is_some() {
            self.seed = seed.clone();
            self.rebuild();
            crate::store::settings_store::update(|s| s.seed = seed);
        }
    }

    fn rebuild(&mut self) {
        if let Some(scheme) = build_scheme(&self.seed, self.dark) {
            self.scheme = scheme;
        }
        self.changed();
    }

    pub fn primary(&self) -> String {
        format!("#{}", self.scheme.primary.as_hex())
    }

    pub fn on_primary(&self) -> String {
        format!("#{}", self.scheme.on_primary.as_hex())
    }

    pub fn primary_container(&self) -> String {
        format!("#{}", self.scheme.primary_container.as_hex())
    }

    pub fn on_primary_container(&self) -> String {
        format!("#{}", self.scheme.on_primary_container.as_hex())
    }

    pub fn secondary(&self) -> String {
        format!("#{}", self.scheme.secondary.as_hex())
    }

    pub fn on_secondary(&self) -> String {
        format!("#{}", self.scheme.on_secondary.as_hex())
    }

    pub fn secondary_container(&self) -> String {
        format!("#{}", self.scheme.secondary_container.as_hex())
    }

    pub fn on_secondary_container(&self) -> String {
        format!("#{}", self.scheme.on_secondary_container.as_hex())
    }

    pub fn tertiary(&self) -> String {
        format!("#{}", self.scheme.tertiary.as_hex())
    }

    pub fn tertiary_container(&self) -> String {
        format!("#{}", self.scheme.tertiary_container.as_hex())
    }

    pub fn on_tertiary_container(&self) -> String {
        format!("#{}", self.scheme.on_tertiary_container.as_hex())
    }

    pub fn error(&self) -> String {
        format!("#{}", self.scheme.error.as_hex())
    }

    pub fn error_container(&self) -> String {
        format!("#{}", self.scheme.error_container.as_hex())
    }

    pub fn surface(&self) -> String {
        format!("#{}", self.scheme.surface.as_hex())
    }

    pub fn on_surface(&self) -> String {
        format!("#{}", self.scheme.on_surface.as_hex())
    }

    pub fn surface_variant(&self) -> String {
        format!("#{}", self.scheme.surface_variant.as_hex())
    }

    pub fn on_surface_variant(&self) -> String {
        format!("#{}", self.scheme.on_surface_variant.as_hex())
    }

    pub fn surface_container_low(&self) -> String {
        format!("#{}", self.scheme.surface_container_low.as_hex())
    }

    pub fn surface_container(&self) -> String {
        format!("#{}", self.scheme.surface_container.as_hex())
    }

    pub fn surface_container_high(&self) -> String {
        format!("#{}", self.scheme.surface_container_high.as_hex())
    }

    pub fn surface_container_highest(&self) -> String {
        format!("#{}", self.scheme.surface_container_highest.as_hex())
    }

    pub fn outline(&self) -> String {
        format!("#{}", self.scheme.outline.as_hex())
    }

    pub fn outline_variant(&self) -> String {
        format!("#{}", self.scheme.outline_variant.as_hex())
    }

    pub fn inverse_surface(&self) -> String {
        format!("#{}", self.scheme.inverse_surface.as_hex())
    }

    pub fn inverse_on_surface(&self) -> String {
        format!("#{}", self.scheme.inverse_on_surface.as_hex())
    }

    pub fn inverse_primary(&self) -> String {
        format!("#{}", self.scheme.inverse_primary.as_hex())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Reference values generated by avio's @material/material-color-utilities for the default seed
    #[test]
    fn matches_avio_palette() {
        let mut theme = Theme::default();
        assert_eq!(theme.surface(), "#faf8ff");
        assert_eq!(theme.primary(), "#0056c9");
        assert_eq!(theme.surface_container(), "#ecedf9");

        theme.dark = true;
        theme.scheme = build_scheme(DEFAULT_SEED, true).unwrap();
        assert_eq!(theme.surface(), "#10131b");
        assert_eq!(theme.primary(), "#b0c6ff");
        assert_eq!(theme.surface_container(), "#1d1f27");
    }
}
