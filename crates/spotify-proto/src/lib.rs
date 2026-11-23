#![allow(clippy::all, clippy::pedantic, missing_docs, rustdoc::all)]

pub use prost::Message;

pub mod spotify {
    pub mod metadata {
        include!(concat!(env!("OUT_DIR"), "/spotify.metadata.rs"));
        pub mod contentratings {
            pub mod v1 {
                include!(concat!(
                    env!("OUT_DIR"),
                    "/spotify.metadata.contentratings.v1.rs"
                ));
            }
        }
    }
    pub mod extendedmetadata {
        include!(concat!(env!("OUT_DIR"), "/spotify.extendedmetadata.rs"));
        pub mod audiofiles {
            include!(concat!(
                env!("OUT_DIR"),
                "/spotify.extendedmetadata.audiofiles.rs"
            ));
        }
    }
    pub mod connectstate {
        include!(concat!(env!("OUT_DIR"), "/spotify.connectstate.rs"));
        pub mod devices {
            include!(concat!(env!("OUT_DIR"), "/spotify.connectstate.devices.rs"));
        }
    }
    pub mod common {
        pub mod media {
            include!(concat!(env!("OUT_DIR"), "/spotify.common.media.rs"));
        }
    }
    pub mod player {
        pub mod proto {
            include!(concat!(env!("OUT_DIR"), "/spotify.player.proto.rs"));
            pub mod transfer {
                include!(concat!(
                    env!("OUT_DIR"),
                    "/spotify.player.proto.transfer.rs"
                ));
            }
        }
    }
    pub mod download {
        pub mod proto {
            include!(concat!(env!("OUT_DIR"), "/spotify.download.proto.rs"));
        }
    }
}

pub use spotify::{connectstate, extendedmetadata, metadata};
