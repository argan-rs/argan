A form data type.

 `Form` consumes the request body and deserializes it as type `T`. `T` must be a type
 that implements [`serde::Deserialize`].

 ```
 use argan::data::form::Form;
 use serde::Deserialize;

 #[derive(Deserialize)]
 struct Person {
   first_name: String,
   last_name: String,
   age: u8,
 }

 async fn add_person(Form(person): Form<Person>) {
   // ...
 }
 ```

 By default, `Form` limits the body size to 2MiB. The body size limit can be changed by
 specifying the SIZE_LIMIT const type parameter.

 ```
 use argan::data::form::Form;
 use serde::Deserialize;

 #[derive(Deserialize)]
 struct SurveyData {
   // ...
 }

 async fn save_survey_data(Form(survey_data): Form<SurveyData, { 512 * 1024 }>) {
   // ...
 }
 ```

 Usually, `GET` and `HEAD` requests carry the data in a query string. With these
 requests, data can be obtained via [`RequestHead::query_params_as<T>`]. For this
 to work `"query-params"` feature flag must be enabled.

[`RequestHead::query_params_as<T>`]: crate::request::RequestHead
