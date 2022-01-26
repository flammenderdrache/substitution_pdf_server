use actix_web::{get, HttpResponse, Responder, web};
use crate::{JSON_HANDLER, Schoolday};

#[get("/{schoolday}")]
pub async fn get_schoolday_pdf_json(day: web::Path<Schoolday>) -> impl Responder {
	if let Some(json) = JSON_HANDLER.get_json(*day).await {
		return HttpResponse::Ok()
			.content_type("application/json")
			.body(json);
	}

	HttpResponse::NoContent()
		.append_header(("Retry-After", "120"))
		.finish()
}
