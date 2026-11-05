use refract::{prelude::*, App, ElementKind, Layout};
use refract_test::TestHost;

#[test]
fn empty_view_can_gain_children_without_replacing_its_box() {
    let mut app = App::new(refract::view());
    let mut host = TestHost::default();
    host.flush(&mut app).unwrap();
    let root = host.children[&None][0];
    assert_eq!(host.elements.len(), 1);
    assert_eq!(host.elements[&root].kind, ElementKind::Box);
    assert_eq!(host.elements[&root].style.layout, Layout::Box);

    app.set_view(view().child(text("First")).children([text("Second")]));
    host.flush(&mut app).unwrap();
    assert_eq!(host.children[&None], [root]);
    let children = &host.children[&Some(root)];
    assert_eq!(children.len(), 2);
    assert_eq!(host.elements[&children[0]].text, "First");
    assert_eq!(host.elements[&children[1]].text, "Second");
}

#[cfg(feature = "macros")]
mod macro_compatibility {
    use super::*;
    use refract::view;

    #[view]
    fn card(_cx: &mut ViewContext<'_>) -> impl View {
        view().child(text("Hello"))
    }

    #[test]
    fn constructor_and_attribute_share_an_import() {
        let mut app = App::new(card());
        let mut host = TestHost::default();
        host.flush(&mut app).unwrap();
        let root = host.children[&None][0];
        assert_eq!(host.elements[&root].kind, ElementKind::Box);
        assert_eq!(host.text(), "Hello");
    }
}
