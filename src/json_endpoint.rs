use actix_web::{get, HttpResponse, Responder, web};
use crate::Schoolday;
use crate::PDF_JSON_STORE;

#[get("/{schoolday}")]
pub async fn get_schoolday_pdf_json(day: web::Path<Schoolday>) -> impl Responder {
	let store = PDF_JSON_STORE.read().await;
	if let Some(json) = store.get(&day) {
		return HttpResponse::Ok()
			.content_type("application/json")
			.body(json.clone());
	}

	HttpResponse::NoContent()
		.append_header(("Retry-After", "120"))
		.finish()
}
