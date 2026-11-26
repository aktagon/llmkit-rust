// Code generated — DO NOT EDIT.

/// The bytes that must sit at `offset`.
pub struct ImageSignaturePart {
    pub offset: usize,
    pub bytes: &'static [u8],
}

/// One image format: its media type, other names callers state for it, its
/// file extension and its byte signatures. A format matches when every part
/// of any one signature matches. A format with no signature is typed by a
/// handwritten text check.
pub struct ImageFormatDef {
    pub media_type: &'static str,
    pub aliases: &'static [&'static str],
    pub extension: &'static str,
    pub signatures: &'static [&'static [ImageSignaturePart]],
}

/// The image format table, in media type order.
pub const IMAGE_FORMATS: &[ImageFormatDef] = &[
    ImageFormatDef {
        media_type: "image/avif",
        aliases: &[],
        extension: ".avif",
        signatures: &[
            // ftyp, major brand avif
            &[
                ImageSignaturePart {
                    offset: 4,
                    bytes: &[0x66, 0x74, 0x79, 0x70],
                },
                ImageSignaturePart {
                    offset: 8,
                    bytes: &[0x61, 0x76, 0x69, 0x66],
                },
            ],
            // ftyp, major brand avis
            &[
                ImageSignaturePart {
                    offset: 4,
                    bytes: &[0x66, 0x74, 0x79, 0x70],
                },
                ImageSignaturePart {
                    offset: 8,
                    bytes: &[0x61, 0x76, 0x69, 0x73],
                },
            ],
        ],
    },
    ImageFormatDef {
        media_type: "image/gif",
        aliases: &[],
        extension: ".gif",
        signatures: &[
            // GIF87a
            &[ImageSignaturePart {
                offset: 0,
                bytes: &[0x47, 0x49, 0x46, 0x38, 0x37, 0x61],
            }],
            // GIF89a
            &[ImageSignaturePart {
                offset: 0,
                bytes: &[0x47, 0x49, 0x46, 0x38, 0x39, 0x61],
            }],
        ],
    },
    ImageFormatDef {
        media_type: "image/heic",
        aliases: &[],
        extension: ".heic",
        signatures: &[
            // ftyp, major brand heic
            &[
                ImageSignaturePart {
                    offset: 4,
                    bytes: &[0x66, 0x74, 0x79, 0x70],
                },
                ImageSignaturePart {
                    offset: 8,
                    bytes: &[0x68, 0x65, 0x69, 0x63],
                },
            ],
            // ftyp, major brand heim
            &[
                ImageSignaturePart {
                    offset: 4,
                    bytes: &[0x66, 0x74, 0x79, 0x70],
                },
                ImageSignaturePart {
                    offset: 8,
                    bytes: &[0x68, 0x65, 0x69, 0x6d],
                },
            ],
            // ftyp, major brand heis
            &[
                ImageSignaturePart {
                    offset: 4,
                    bytes: &[0x66, 0x74, 0x79, 0x70],
                },
                ImageSignaturePart {
                    offset: 8,
                    bytes: &[0x68, 0x65, 0x69, 0x73],
                },
            ],
            // ftyp, major brand heix
            &[
                ImageSignaturePart {
                    offset: 4,
                    bytes: &[0x66, 0x74, 0x79, 0x70],
                },
                ImageSignaturePart {
                    offset: 8,
                    bytes: &[0x68, 0x65, 0x69, 0x78],
                },
            ],
        ],
    },
    ImageFormatDef {
        media_type: "image/jpeg",
        aliases: &["image/jpg"],
        extension: ".jpg",
        signatures: &[
            // SOI and the next marker
            &[ImageSignaturePart {
                offset: 0,
                bytes: &[0xff, 0xd8, 0xff],
            }],
        ],
    },
    ImageFormatDef {
        media_type: "image/png",
        aliases: &[],
        extension: ".png",
        signatures: &[
            // PNG signature
            &[ImageSignaturePart {
                offset: 0,
                bytes: &[0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a],
            }],
        ],
    },
    ImageFormatDef {
        media_type: "image/svg+xml",
        aliases: &[],
        extension: ".svg",
        signatures: &[],
    },
    ImageFormatDef {
        media_type: "image/webp",
        aliases: &[],
        extension: ".webp",
        signatures: &[
            // RIFF container, WEBP
            &[
                ImageSignaturePart {
                    offset: 0,
                    bytes: &[0x52, 0x49, 0x46, 0x46],
                },
                ImageSignaturePart {
                    offset: 8,
                    bytes: &[0x57, 0x45, 0x42, 0x50],
                },
            ],
        ],
    },
];
