use yu_core::{TextAttrs, TextStyle};

use super::{BlockContext, Extension, ExtensionOutput};
use crate::BlockKind;

pub struct FrontMatter;

impl Extension for FrontMatter {
    fn name(&self) -> &'static str {
        "front-matter"
    }

    fn decorate(&self, cx: &BlockContext<'_>, out: &mut ExtensionOutput) {
        if cx.block().kind() == BlockKind::FrontMatter {
            // Metadata remains directly editable, including delimiters. Never
            // deserialize and serialize YAML just to display or save a file.
            let style = out.style(TextAttrs::new(TextStyle::Code));
            out.mark(cx.range(), style);
        }
    }
}
