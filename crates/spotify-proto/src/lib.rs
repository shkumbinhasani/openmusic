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
    pub mod playbacksettings {
        include!(concat!(env!("OUT_DIR"), "/spotify.playbacksettings.rs"));
        pub mod pubsub {
            include!(concat!(
                env!("OUT_DIR"),
                "/spotify.playbacksettings.pubsub.rs"
            ));
        }
    }
    pub mod collection {
        pub mod proto {
            pub mod v2 {
                include!(concat!(env!("OUT_DIR"), "/spotify.collection.proto.v2.rs"));
            }
        }
        pub mod itemdata {
            include!(concat!(env!("OUT_DIR"), "/spotify.collection.itemdata.rs"));
        }
    }
    pub mod list_collection {
        pub mod save_source {
            pub mod proto {
                include!(concat!(
                    env!("OUT_DIR"),
                    "/spotify.list_collection.save_source.proto.rs"
                ));
            }
        }
    }
    pub mod canvaz {
        pub mod cache {
            include!(concat!(env!("OUT_DIR"), "/spotify.canvaz.cache.rs"));
        }
    }
    pub mod bumblebee {
        pub mod video_associations {
            pub mod v1 {
                include!(concat!(
                    env!("OUT_DIR"),
                    "/spotify.bumblebee.video_associations.v1.rs"
                ));
            }
        }
    }
    pub mod playlist4 {
        pub mod proto {
            include!(concat!(env!("OUT_DIR"), "/spotify.playlist4.proto.rs"));
        }
    }
    pub mod playlist_permission {
        pub mod proto {
            include!(concat!(
                env!("OUT_DIR"),
                "/spotify.playlist_permission.proto.rs"
            ));
        }
    }
    pub mod playlist {
        pub mod signal {
            pub mod proto {
                include!(concat!(
                    env!("OUT_DIR"),
                    "/spotify.playlist.signal.proto.rs"
                ));
            }
        }
    }
    pub mod lens {
        pub mod proto {
            include!(concat!(env!("OUT_DIR"), "/spotify.lens.proto.rs"));
        }
    }
    pub mod list {
        pub mod v1 {
            pub mod model {
                pub mod attributes {
                    include!(concat!(
                        env!("OUT_DIR"),
                        "/spotify.list.v1.model.attributes.rs"
                    ));
                }
            }
        }
    }
    pub mod automix {
        pub mod proto {
            include!(concat!(env!("OUT_DIR"), "/spotify.automix.proto.rs"));
        }
    }
}

pub use spotify::{
    automix, bumblebee, canvaz, collection, connectstate, extendedmetadata, metadata,
    playbacksettings, playlist4,
};
