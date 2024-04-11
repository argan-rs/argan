use super::{IntoLayer, Layer};

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

// --------------------------------------------------
// LayerStack

#[derive(Clone)]
pub struct LayerStack<Outer, Inner>(Outer, Inner);

impl<Outer, Inner, H> Layer<H> for LayerStack<Outer, Inner>
where
	Outer: Layer<Inner::Handler>,
	Inner: Layer<H>,
{
	type Handler = Outer::Handler;

	fn wrap(&self, handler: H) -> Self::Handler {
		self.0.wrap(self.1.wrap(handler))
	}
}

macro_rules! stack_layer_type {
	($l1:ident, $($l:ident,)+ ($ll:ident)) => {
		LayerStack<$l1, stack_layer_type!($($l,)+ ($ll))>
	};
	($l:ident, ($ll:ident)) => {
		LayerStack<$l, $ll>
	};
}

impl<L1, L2, H> IntoLayer<(L1, L2), H> for (L1, L2)
where
	L1: Layer<L2::Handler>,
	L2: Layer<H>,
{
	type Layer = stack_layer_type!(L1, (L2));

	fn into_layer(self) -> Self::Layer {
		let (l1, l2) = self;

		LayerStack(l1, l2)
	}
}

impl<L1, L2, L3, H> IntoLayer<(L1, L2, L3), H> for (L1, L2, L3)
where
	L1: Layer<L2::Handler>,
	L2: Layer<L3::Handler>,
	L3: Layer<H>,
{
	type Layer = stack_layer_type!(L1, L2, (L3));

	fn into_layer(self) -> Self::Layer {
		let (l1, l2, l3) = self;

		LayerStack(l1, (l2, l3).into_layer())
	}
}

impl<L1, L2, L3, L4, H> IntoLayer<(L1, L2, L3, L4), H> for (L1, L2, L3, L4)
where
	L1: Layer<L2::Handler>,
	L2: Layer<L3::Handler>,
	L3: Layer<L4::Handler>,
	L4: Layer<H>,
{
	type Layer = stack_layer_type!(L1, L2, L3, (L4));

	fn into_layer(self) -> Self::Layer {
		let (l1, l2, l3, l4) = self;

		LayerStack(l1, (l2, l3, l4).into_layer())
	}
}

impl<L1, L2, L3, L4, L5, H> IntoLayer<(L1, L2, L3, L4, L5), H> for (L1, L2, L3, L4, L5)
where
	L1: Layer<L2::Handler>,
	L2: Layer<L3::Handler>,
	L3: Layer<L4::Handler>,
	L4: Layer<L5::Handler>,
	L5: Layer<H>,
{
	type Layer = stack_layer_type!(L1, L2, L3, L4, (L5));

	fn into_layer(self) -> Self::Layer {
		let (l1, l2, l3, l4, l5) = self;

		LayerStack(l1, (l2, l3, l4, l5).into_layer())
	}
}

impl<L1, L2, L3, L4, L5, L6, H> IntoLayer<(L1, L2, L3, L4, L5, L6), H> for (L1, L2, L3, L4, L5, L6)
where
	L1: Layer<L2::Handler>,
	L2: Layer<L3::Handler>,
	L3: Layer<L4::Handler>,
	L4: Layer<L5::Handler>,
	L5: Layer<L6::Handler>,
	L6: Layer<H>,
{
	type Layer = stack_layer_type!(L1, L2, L3, L4, L5, (L6));

	fn into_layer(self) -> Self::Layer {
		let (l1, l2, l3, l4, l5, l6) = self;

		LayerStack(l1, (l2, l3, l4, l5, l6).into_layer())
	}
}

impl<L1, L2, L3, L4, L5, L6, L7, H> IntoLayer<(L1, L2, L3, L4, L5, L6, L7), H>
	for (L1, L2, L3, L4, L5, L6, L7)
where
	L1: Layer<L2::Handler>,
	L2: Layer<L3::Handler>,
	L3: Layer<L4::Handler>,
	L4: Layer<L5::Handler>,
	L5: Layer<L6::Handler>,
	L6: Layer<L7::Handler>,
	L7: Layer<H>,
{
	type Layer = stack_layer_type!(L1, L2, L3, L4, L5, L6, (L7));

	fn into_layer(self) -> Self::Layer {
		let (l1, l2, l3, l4, l5, l6, l7) = self;

		LayerStack(l1, (l2, l3, l4, l5, l6, l7).into_layer())
	}
}

impl<L1, L2, L3, L4, L5, L6, L7, L8, H> IntoLayer<(L1, L2, L3, L4, L5, L6, L7, L8), H>
	for (L1, L2, L3, L4, L5, L6, L7, L8)
where
	L1: Layer<L2::Handler>,
	L2: Layer<L3::Handler>,
	L3: Layer<L4::Handler>,
	L4: Layer<L5::Handler>,
	L5: Layer<L6::Handler>,
	L6: Layer<L7::Handler>,
	L7: Layer<L8::Handler>,
	L8: Layer<H>,
{
	type Layer = stack_layer_type!(L1, L2, L3, L4, L5, L6, L7, (L8));

	fn into_layer(self) -> Self::Layer {
		let (l1, l2, l3, l4, l5, l6, l7, l8) = self;

		LayerStack(l1, (l2, l3, l4, l5, l6, l7, l8).into_layer())
	}
}

impl<L1, L2, L3, L4, L5, L6, L7, L8, L9, H> IntoLayer<(L1, L2, L3, L4, L5, L6, L7, L8, L9), H>
	for (L1, L2, L3, L4, L5, L6, L7, L8, L9)
where
	L1: Layer<L2::Handler>,
	L2: Layer<L3::Handler>,
	L3: Layer<L4::Handler>,
	L4: Layer<L5::Handler>,
	L5: Layer<L6::Handler>,
	L6: Layer<L7::Handler>,
	L7: Layer<L8::Handler>,
	L8: Layer<L9::Handler>,
	L9: Layer<H>,
{
	type Layer = stack_layer_type!(L1, L2, L3, L4, L5, L6, L7, L8, (L9));

	fn into_layer(self) -> Self::Layer {
		let (l1, l2, l3, l4, l5, l6, l7, l8, l9) = self;

		LayerStack(l1, (l2, l3, l4, l5, l6, l7, l8, l9).into_layer())
	}
}

impl<L1, L2, L3, L4, L5, L6, L7, L8, L9, L10, H>
	IntoLayer<(L1, L2, L3, L4, L5, L6, L7, L8, L9, L10), H>
	for (L1, L2, L3, L4, L5, L6, L7, L8, L9, L10)
where
	L1: Layer<L2::Handler>,
	L2: Layer<L3::Handler>,
	L3: Layer<L4::Handler>,
	L4: Layer<L5::Handler>,
	L5: Layer<L6::Handler>,
	L6: Layer<L7::Handler>,
	L7: Layer<L8::Handler>,
	L8: Layer<L9::Handler>,
	L9: Layer<L10::Handler>,
	L10: Layer<H>,
{
	type Layer = stack_layer_type!(L1, L2, L3, L4, L5, L6, L7, L8, L9, (L10));

	fn into_layer(self) -> Self::Layer {
		let (l1, l2, l3, l4, l5, l6, l7, l8, l9, l10) = self;

		LayerStack(l1, (l2, l3, l4, l5, l6, l7, l8, l9, l10).into_layer())
	}
}

impl<L1, L2, L3, L4, L5, L6, L7, L8, L9, L10, L11, H>
	IntoLayer<(L1, L2, L3, L4, L5, L6, L7, L8, L9, L10, L11), H>
	for (L1, L2, L3, L4, L5, L6, L7, L8, L9, L10, L11)
where
	L1: Layer<L2::Handler>,
	L2: Layer<L3::Handler>,
	L3: Layer<L4::Handler>,
	L4: Layer<L5::Handler>,
	L5: Layer<L6::Handler>,
	L6: Layer<L7::Handler>,
	L7: Layer<L8::Handler>,
	L8: Layer<L9::Handler>,
	L9: Layer<L10::Handler>,
	L10: Layer<L11::Handler>,
	L11: Layer<H>,
{
	type Layer = stack_layer_type!(L1, L2, L3, L4, L5, L6, L7, L8, L9, L10, (L11));

	fn into_layer(self) -> Self::Layer {
		let (l1, l2, l3, l4, l5, l6, l7, l8, l9, l10, l11) = self;

		LayerStack(l1, (l2, l3, l4, l5, l6, l7, l8, l9, l10, l11).into_layer())
	}
}

impl<L1, L2, L3, L4, L5, L6, L7, L8, L9, L10, L11, L12, H>
	IntoLayer<(L1, L2, L3, L4, L5, L6, L7, L8, L9, L10, L11, L12), H>
	for (L1, L2, L3, L4, L5, L6, L7, L8, L9, L10, L11, L12)
where
	L1: Layer<L2::Handler>,
	L2: Layer<L3::Handler>,
	L3: Layer<L4::Handler>,
	L4: Layer<L5::Handler>,
	L5: Layer<L6::Handler>,
	L6: Layer<L7::Handler>,
	L7: Layer<L8::Handler>,
	L8: Layer<L9::Handler>,
	L9: Layer<L10::Handler>,
	L10: Layer<L11::Handler>,
	L11: Layer<L12::Handler>,
	L12: Layer<H>,
{
	type Layer = stack_layer_type!(L1, L2, L3, L4, L5, L6, L7, L8, L9, L10, L11, (L12));

	fn into_layer(self) -> Self::Layer {
		let (l1, l2, l3, l4, l5, l6, l7, l8, l9, l10, l11, l12) = self;

		LayerStack(
			l1,
			(l2, l3, l4, l5, l6, l7, l8, l9, l10, l11, l12).into_layer(),
		)
	}
}

impl<L1, L2, L3, L4, L5, L6, L7, L8, L9, L10, L11, L12, L13, H>
	IntoLayer<(L1, L2, L3, L4, L5, L6, L7, L8, L9, L10, L11, L12, L13), H>
	for (L1, L2, L3, L4, L5, L6, L7, L8, L9, L10, L11, L12, L13)
where
	L1: Layer<L2::Handler>,
	L2: Layer<L3::Handler>,
	L3: Layer<L4::Handler>,
	L4: Layer<L5::Handler>,
	L5: Layer<L6::Handler>,
	L6: Layer<L7::Handler>,
	L7: Layer<L8::Handler>,
	L8: Layer<L9::Handler>,
	L9: Layer<L10::Handler>,
	L10: Layer<L11::Handler>,
	L11: Layer<L12::Handler>,
	L12: Layer<L13::Handler>,
	L13: Layer<H>,
{
	type Layer = stack_layer_type!(L1, L2, L3, L4, L5, L6, L7, L8, L9, L10, L11, L12, (L13));

	fn into_layer(self) -> Self::Layer {
		let (l1, l2, l3, l4, l5, l6, l7, l8, l9, l10, l11, l12, l13) = self;

		LayerStack(
			l1,
			(l2, l3, l4, l5, l6, l7, l8, l9, l10, l11, l12, l13).into_layer(),
		)
	}
}

#[rustfmt::skip]
impl<L1, L2, L3, L4, L5, L6, L7, L8, L9, L10, L11, L12, L13, L14, H>
	IntoLayer<(L1, L2, L3, L4, L5, L6, L7, L8, L9, L10, L11, L12, L13, L14), H>
	for (L1, L2, L3, L4, L5, L6, L7, L8, L9, L10, L11, L12, L13, L14)
where
	L1: Layer<L2::Handler>,
	L2: Layer<L3::Handler>,
	L3: Layer<L4::Handler>,
	L4: Layer<L5::Handler>,
	L5: Layer<L6::Handler>,
	L6: Layer<L7::Handler>,
	L7: Layer<L8::Handler>,
	L8: Layer<L9::Handler>,
	L9: Layer<L10::Handler>,
	L10: Layer<L11::Handler>,
	L11: Layer<L12::Handler>,
	L12: Layer<L13::Handler>,
	L13: Layer<L14::Handler>,
	L14: Layer<H>,
{
	type Layer = stack_layer_type!(L1, L2, L3, L4, L5, L6, L7, L8, L9, L10, L11, L12, L13, (L14));

	fn into_layer(self) -> Self::Layer {
		let (l1, l2, l3, l4, l5, l6, l7, l8, l9, l10, l11, l12, l13, l14) = self;

		LayerStack(l1, (l2, l3, l4, l5, l6, l7, l8, l9, l10, l11, l12, l13, l14).into_layer())
	}
}

#[rustfmt::skip]
impl<L1, L2, L3, L4, L5, L6, L7, L8, L9, L10, L11, L12, L13, L14, L15, H>
	IntoLayer<(L1, L2, L3, L4, L5, L6, L7, L8, L9, L10, L11, L12, L13, L14, L15), H>
	for (L1, L2, L3, L4, L5, L6, L7, L8, L9, L10, L11, L12, L13, L14, L15)
where
	L1: Layer<L2::Handler>,
	L2: Layer<L3::Handler>,
	L3: Layer<L4::Handler>,
	L4: Layer<L5::Handler>,
	L5: Layer<L6::Handler>,
	L6: Layer<L7::Handler>,
	L7: Layer<L8::Handler>,
	L8: Layer<L9::Handler>,
	L9: Layer<L10::Handler>,
	L10: Layer<L11::Handler>,
	L11: Layer<L12::Handler>,
	L12: Layer<L13::Handler>,
	L13: Layer<L14::Handler>,
	L14: Layer<L15::Handler>,
	L15: Layer<H>,
{
	type Layer = stack_layer_type!(
		L1, L2, L3, L4, L5, L6, L7, L8, L9, L10, L11, L12, L13, L14, (L15)
	);

	fn into_layer(self) -> Self::Layer {
		let (l1, l2, l3, l4, l5, l6, l7, l8, l9, l10, l11, l12, l13, l14, l15) = self;

		LayerStack(l1, (l2, l3, l4, l5, l6, l7, l8, l9, l10, l11, l12, l13, l14, l15).into_layer())
	}
}

#[rustfmt::skip]
impl<L1, L2, L3, L4, L5, L6, L7, L8, L9, L10, L11, L12, L13, L14, L15, L16, H>
	IntoLayer<(L1, L2, L3, L4, L5, L6, L7, L8, L9, L10, L11, L12, L13, L14, L15, L16), H>
	for (L1, L2, L3, L4, L5, L6, L7, L8, L9, L10, L11, L12, L13, L14, L15, L16)
where
	L1: Layer<L2::Handler>,
	L2: Layer<L3::Handler>,
	L3: Layer<L4::Handler>,
	L4: Layer<L5::Handler>,
	L5: Layer<L6::Handler>,
	L6: Layer<L7::Handler>,
	L7: Layer<L8::Handler>,
	L8: Layer<L9::Handler>,
	L9: Layer<L10::Handler>,
	L10: Layer<L11::Handler>,
	L11: Layer<L12::Handler>,
	L12: Layer<L13::Handler>,
	L13: Layer<L14::Handler>,
	L14: Layer<L15::Handler>,
	L15: Layer<L16::Handler>,
	L16: Layer<H>,
{
	type Layer = stack_layer_type!(
		L1, L2, L3, L4, L5, L6, L7, L8, L9, L10, L11, L12, L13, L14, L15, (L16)
	);

	fn into_layer(self) -> Self::Layer {
		let (l1, l2, l3, l4, l5, l6, l7, l8, l9, l10, l11, l12, l13, l14, l15, l16) = self;

		LayerStack(
			l1,
			(l2, l3, l4, l5, l6, l7, l8, l9, l10, l11, l12, l13, l14, l15, l16).into_layer(),
		)
	}
}

// --------------------------------------------------
