//! Utilities for components and component properties.

use std::fmt;

use crate::reactive::*;

/// Runs the given closure inside a new component scope. In other words, this does the following:
/// * Create a new untracked scope (see [`untrack`]).
/// * Call the closure `f` passed to this function.
#[doc(hidden)]
pub fn component_scope<T>(f: impl FnOnce() -> T) -> T {
    untrack(f)
}

/// A trait that is implemented automatically by the `Props` derive macro.
///
/// This is used when constructing components in the `view!` macro.
///
/// # Example
/// Deriving an implementation and using the builder to construct an instance of the struct:
/// ```ignore
/// # use sycamore::prelude::*;
/// #[derive(Props)]
/// struct ButtonProps {
///     color: String,
///     disabled: bool,
/// }
///
/// let builder = <ButtonProps as Props>::builder();
/// let button_props = builder.color("red".to_string()).disabled(false).build();
/// ```
pub trait Props {
    /// The type of the builder. This allows getting the builder type when the name is unknown (e.g.
    /// in a macro).
    type Builder;
    /// Returns the builder for the type.
    /// The builder should be automatically generated using the `Props` derive macro.
    fn builder() -> Self::Builder;
}

/// Make sure that the `Props` trait is implemented for `()` so that components without props can be
/// thought as accepting props of type `()`.
impl Props for () {
    type Builder = UnitBuilder;
    fn builder() -> Self::Builder {
        UnitBuilder
    }
}

#[doc(hidden)]
#[derive(Debug)]
pub struct UnitBuilder;

impl UnitBuilder {
    pub fn build(self) {}
}

/// A trait that is automatically implemented by all components.
pub trait Component<T: Props, V, S> {
    /// Instantiate the component with the given props and reactive scope.
    fn create(self, props: T) -> V;
}
impl<F, T: Props, V> Component<T, V, ((),)> for F
where
    F: FnOnce(T) -> V,
{
    fn create(self, props: T) -> V {
        self(props)
    }
}
impl<F, V> Component<(), V, ()> for F
where
    F: FnOnce() -> V,
{
    fn create(self, _props: ()) -> V {
        self()
    }
}

/// Get the builder for the component function.
#[doc(hidden)]
pub fn element_like_component_builder<T: Props, V, S>(_f: &impl Component<T, V, S>) -> T::Builder {
    T::builder()
}

/// A special property type to allow the component to accept children.
///
/// Add a field called `children` of this type to your properties struct.
///
/// # Example
/// ```ignore
/// # use sycamore::prelude::*;
/// #[derive(Props)]
/// struct RowProps {
///     width: i32,
///     children: Children,
/// }
///
/// #[component]
/// fn Row(props: RowProps) -> View {
///     // Convert the `Children` into a `View`.
///     let children = props.children.call();
///     view! {
///         div {
///             (children)
///         }
///     }
/// }
///
/// # #[component]
/// # fn App() -> View {
/// // Using `Row` somewhere else in your app:
/// view! {
///     Row(width=10) {
///         p { "This is a child node." }
///     }
/// }
/// # }
/// ```
pub struct Children<V> {
    f: Box<dyn FnOnce() -> V>,
}
impl<V> fmt::Debug for Children<V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Children").finish()
    }
}

impl<F, V> From<F> for Children<V>
where
    F: FnOnce() -> V + 'static,
{
    fn from(f: F) -> Self {
        Self { f: Box::new(f) }
    }
}

impl<V: Default + 'static> Default for Children<V> {
    fn default() -> Self {
        Self {
            f: Box::new(V::default),
        }
    }
}

impl<V> Children<V> {
    /// Instantiates the child view.
    pub fn call(self) -> V {
        (self.f)()
    }

    /// Create a new [`Children`] from a closure.
    pub fn new(f: impl FnOnce() -> V + 'static) -> Self {
        Self { f: Box::new(f) }
    }
}

#[cfg(test)]
mod tests {
    use crate::framebuffer::Rect;
    use crate::layout::render as render_buffer;
    use crate::prelude::*;
    use crate::reactive::create_root;

    // A plain-fn component parameterised by a `#[derive(Props)]` struct. No
    // `#[component]` attribute yet — the blanket `Component` impl for
    // `FnOnce(Props) -> View` is enough. This validates the keystone: the derive
    // generates the typed builder, `view!`'s component codegen calls the setters,
    // and `Component::create` invokes the fn.
    #[derive(Props)]
    struct GreetingProps {
        name: String,
    }

    #[allow(non_snake_case)]
    fn Greeting(props: GreetingProps) -> View {
        view! {
            p { "Hello, " (props.name) }
        }
    }

    #[test]
    fn derive_props_component_builds_and_invokes_from_view_macro() {
        let mut nodes = Vec::new();
        let root = create_root(|| {
            let view: View = view! {
                Greeting(name = String::from("world"))
            };
            nodes = view.nodes.into_iter().collect();
        });

        let (fb, _index) = render_buffer(&nodes, Rect::new(0, 0, 20, 3), None);
        let painted = fb.to_string();
        assert!(painted.contains("Hello"), "label painted: {painted}");
        assert!(painted.contains("world"), "name prop painted: {painted}");
        root.dispose();
    }

    // Optional + defaulted fields: `#[prop(default)]` and `Option<T>` auto-strip
    // must both flow through the builder.
    #[derive(Props)]
    struct OptionalProps {
        required: i32,
        #[prop(default)]
        defaulted: i32,
        maybe: Option<String>,
    }

    #[allow(non_snake_case)]
    fn Optional(props: OptionalProps) -> View {
        // Compose outside the view! closure so we only consume `props` once.
        let label = format!(
            "{}-{}-{}",
            props.required,
            props.defaulted,
            props.maybe.as_deref().unwrap_or("")
        );
        view! {
            p { (label) }
        }
    }

    #[test]
    fn derive_props_optional_and_default_fields() {
        let mut nodes = Vec::new();
        let root = create_root(|| {
            // `defaulted` omitted (uses default 0); `maybe` omitted (auto-stripped Option → None).
            let view: View = view! {
                Optional(required = 42)
            };
            nodes = view.nodes.into_iter().collect();
        });

        let (fb, _index) = render_buffer(&nodes, Rect::new(0, 0, 20, 1), None);
        let painted = fb.to_string();
        assert!(painted.contains("42"), "required painted: {painted}");
        // required=42, defaulted=0, maybe="" -> "42-0-"
        assert!(
            painted.contains("42-0-"),
            "default (0) and stripped Option (empty) applied: {painted}"
        );
        root.dispose();
    }

    // The `#[component]` attribute on a sync fn: validates the signature and
    // marks it as a component. Functionally equivalent to a plain fn (the
    // blanket `Component` impl already applies), but this confirms the macro
    // round-trips and stays in scope from the prelude.
    #[derive(Props)]
    struct CounterProps {
        value: i32,
    }

    #[component]
    fn Counter(props: CounterProps) -> View {
        view! {
            p { "count="(props.value) }
        }
    }

    #[test]
    fn component_attribute_on_sync_fn() {
        let mut nodes = Vec::new();
        let root = create_root(|| {
            let view: View = view! {
                Counter(value = 7)
            };
            nodes = view.nodes.into_iter().collect();
        });
        let (fb, _index) = render_buffer(&nodes, Rect::new(0, 0, 20, 1), None);
        let painted = fb.to_string();
        assert!(painted.contains("count=7"), "component fn painted: {painted}");
        root.dispose();
    }

    // `#[component(inline_props)]`: no separate `#[derive(Props)]` struct — the
    // macro synthesises `Banner_Props { title, count }` from the fn params and
    // rewrites the body to destructure it.
    #[component(inline_props)]
    fn Banner(title: String, count: i32) -> View {
        view! {
            p { (title)": "(count) }
        }
    }

    #[test]
    fn component_inline_props_synthesises_props_struct() {
        let mut nodes = Vec::new();
        let root = create_root(|| {
            let view: View = view! {
                Banner(title = String::from("Items"), count = 3)
            };
            nodes = view.nodes.into_iter().collect();
        });
        let (fb, _index) = render_buffer(&nodes, Rect::new(0, 0, 20, 1), None);
        let painted = fb.to_string();
        assert!(painted.contains("Items"), "inline_props title painted: {painted}");
        assert!(painted.contains('3'), "inline_props count painted: {painted}");
        root.dispose();
    }
}
