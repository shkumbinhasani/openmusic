use std::{cell::RefCell, collections::HashMap, sync::Arc};

use gpui::{Image, Img, img};

#[derive(Default)]
struct Drawn {
    frame: u64,
    images: HashMap<u64, (Arc<Image>, u64)>,
}

impl Drawn {
    fn record(&mut self, image: &Arc<Image>) {
        self.images.insert(image.id(), (image.clone(), self.frame));
    }

    fn forget(&mut self, image: &Image) -> bool {
        self.images.remove(&image.id()).is_some()
    }
}

thread_local! {
    static DRAWN: RefCell<Drawn> = RefCell::default();
}

pub fn next_frame() {
    DRAWN.with_borrow_mut(|drawn| drawn.frame += 1);
}

pub fn cover(image: Arc<Image>) -> Img {
    DRAWN.with_borrow_mut(|drawn| drawn.record(&image));
    img(image)
}

pub fn forget(image: &Image) -> bool {
    DRAWN.with_borrow_mut(|drawn| drawn.forget(image))
}

pub fn drain() -> Vec<Arc<Image>> {
    DRAWN.with_borrow_mut(|drawn| drawn.images.drain().map(|(_, (image, _))| image).collect())
}

#[cfg(test)]
mod tests {
    use gpui::ImageFormat;

    use super::*;

    #[test]
    fn remembers_which_images_were_drawn() {
        let mut drawn = Drawn::default();
        let (shown, hidden) = (
            Arc::new(Image::from_bytes(ImageFormat::Png, vec![1])),
            Arc::new(Image::from_bytes(ImageFormat::Png, vec![2])),
        );
        drawn.record(&shown);
        assert!(drawn.forget(&shown));
        assert!(!drawn.forget(&shown));
        assert!(!drawn.forget(&hidden));
    }
}
