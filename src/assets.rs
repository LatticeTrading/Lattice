//! NSH compile-time assets. The SVG is inlined by the shared shell; fonts are
//! registered by the crate's asset registry before the first frame.

use nsh_core::element::{FontAsset, asset};

#[allow(dead_code)]
pub(crate) const IBM_PLEX_MONO_REGULAR: FontAsset =
    asset!("fonts/ibm-plex-mono/IBMPlexMono-Regular.ttf");
#[allow(dead_code)]
pub(crate) const IBM_PLEX_MONO_MEDIUM: FontAsset =
    asset!("fonts/ibm-plex-mono/IBMPlexMono-Medium.ttf");
#[allow(dead_code)]
pub(crate) const IBM_PLEX_MONO_SEMIBOLD: FontAsset =
    asset!("fonts/ibm-plex-mono/IBMPlexMono-SemiBold.ttf");
#[allow(dead_code)]
pub(crate) const IBM_PLEX_MONO_BOLD: FontAsset = asset!("fonts/ibm-plex-mono/IBMPlexMono-Bold.ttf");
