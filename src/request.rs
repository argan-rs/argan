use super::body::*;

// --------------------------------------------------

pub type Request<B = Incoming> = hyper::Request<B>;

// --------------------------------------------------

pub trait FromRequest<B>
where
	B: Body,
{
	fn from_request(req: Request<B>) -> Self;
}
