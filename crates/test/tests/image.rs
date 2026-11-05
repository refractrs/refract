use refract_core::{prelude::*, App, ElementKind, Patch};
use refract_test::TestHost;

#[test]
fn alternative_text_is_optional_and_updates_the_styled_image_in_place() {
    let mut host = TestHost::default();
    let mut app = App::new(image("photo.png").width(120.0));
    host.flush(&mut app).unwrap();
    let id = host.children[&None][0];
    assert_eq!(host.elements.len(), 1);
    assert_eq!(host.elements[&id].label, None);

    // Labels target the image before and after shared style modifiers.
    for (next, expected) in [
        (
            image("photo.png").label("Before styling").width(120.0),
            Some("Before styling"),
        ),
        (
            image("photo.png").width(120.0).label("After styling"),
            Some("After styling"),
        ),
        (
            image("photo.png").label("Replaced").width(120.0).label(""),
            Some(""),
        ),
        (image("photo.png").width(120.0), None),
    ] {
        app.set_view(next);
        host.flush(&mut app).unwrap();
        assert_eq!(host.children[&None], [id]);
        assert_eq!(host.elements.len(), 1);
        assert_eq!(host.elements[&id].label.as_deref(), expected);
        let patches = &host.updates.last().unwrap().patches;
        assert!(
            matches!(patches.as_slice(), [Patch::Update(target, element)]
            if *target == id && element.kind == ElementKind::Image)
        );
    }
    assert_eq!(host.elements[&id].label, None);

    app.set_view(image("photo.png").label("Photo").padding(8.0));
    host.flush(&mut app).unwrap();
    let wrapper = host.children[&None][0];
    let image = host.children[&Some(wrapper)][0];
    assert_eq!(host.elements[&image].label.as_deref(), Some("Photo"));
    assert_eq!(host.elements[&wrapper].label, None);
}

#[test]
fn image_url_validation_preserves_supported_sources_and_rejects_other_schemes() {
    let mut host = TestHost::default();
    let mut app = App::new(image("assets/photo.png").label("Photo"));
    for source in [
        "assets/photo.png",
        "/photo.png",
        "https://example.com/photo.png",
        "http://example.com/photo.png",
        "blob:https://example.com/image-id",
        "photo.png?variant=small:2",
    ] {
        app.set_view(image(source).label("Photo"));
        host.flush(&mut app).unwrap();
    }

    let accepted = host.elements.clone();
    let revision = host.revision;
    for source in [
        "javascript:alert(1)",
        "data:image/png;base64,AAAA",
        "mailto:person@example.com",
        "tel:123456789",
        "file:///photo.png",
    ] {
        app.set_view(image(source).label("Photo"));
        assert_eq!(
            host.flush(&mut app).unwrap_err().0,
            "unsupported URL scheme",
            "{source}",
        );
        assert_eq!(host.elements, accepted);
        assert_eq!(host.revision, revision);
    }

    // Encoded assets do not pass through URL validation; adapters decode them.
    app.set_view(
        image(ImageSource::embedded(
            ImageFormat::Png,
            b"encoded asset".as_slice(),
        ))
        .label("Embedded photo"),
    );
    host.flush(&mut app).unwrap();
}
