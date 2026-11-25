use std::{cell::RefCell, collections::HashMap, sync::Arc};

use gpui::{Image, Img, img};

const KEPT_FRAMES: u64 = 2;

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

    fn offscreen(&mut self, limit: usize) -> Vec<Arc<Image>> {
        let excess = self.images.len().saturating_sub(limit);
        if excess == 0 {
            return Vec::new();
        }
        let recent = self.frame.saturating_sub(KEPT_FRAMES - 1);
        let mut old: Vec<(u64, u64)> = self
            .images
            .iter()
            .filter(|(_, (_, frame))| *frame < recent)
            .map(|(id, (_, frame))| (*frame, *id))
            .collect();
        old.sort_unstable();
        old.into_iter()
            .take(excess)
            .filter_map(|(_, id)| self.images.remove(&id))
            .map(|(image, _)| image)
            .collect()
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

pub fn offscreen(limit: usize) -> Vec<Arc<Image>> {
    DRAWN.with_borrow_mut(|drawn| drawn.offscreen(limit))
}

pub fn drain() -> Vec<Arc<Image>> {
    DRAWN.with_borrow_mut(|drawn| drawn.images.drain().map(|(_, (image, _))| image).collect())
}

#[cfg(test)]
mod tests {
    use gpui::ImageFormat;

    use super::*;

    fn image(byte: u8) -> Arc<Image> {
        Arc::new(Image::from_bytes(ImageFormat::Png, vec![byte]))
    }

    #[test]
    fn releases_the_longest_unseen_images_beyond_the_limit() {
        let mut drawn = Drawn::default();
        let (old, older, shown) = (image(1), image(2), image(3));
        drawn.record(&older);
        drawn.frame += 1;
        drawn.record(&old);
        drawn.frame += 1;
        drawn.record(&shown);
        assert!(drawn.offscreen(3).is_empty());
        let released = drawn.offscreen(2);
        assert_eq!(released.len(), 1);
        assert_eq!(released[0].id(), older.id());
        drawn.frame += 1;
        drawn.record(&shown);
        assert!(
            drawn
                .offscreen(0)
                .iter()
                .all(|image| image.id() == old.id())
        );
        assert!(drawn.forget(&shown));
        assert!(!drawn.forget(&old));
    }

    #[test]
    fn never_releases_what_the_last_two_frames_drew() {
        let mut drawn = Drawn::default();
        let (previous, current) = (image(1), image(2));
        drawn.record(&previous);
        drawn.frame += 1;
        drawn.record(&current);
        assert!(drawn.offscreen(0).is_empty());
        drawn.frame += 1;
        assert_eq!(drawn.offscreen(0).len(), 1);
    }
}
