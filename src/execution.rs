use crate::solidarity::image::Image;


struct Session<'s> {
    image: &'s mut Image
}