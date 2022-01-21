use actix_web::{get, Responder};

#[get("/{schoolday}")]
pub async fn get_schoolday_pdf_json() -> impl Responder {
	""
}
