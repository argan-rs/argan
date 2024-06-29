A `Json` type to extract and send data as JSON.

`Json` consumes the request body and deserializes it as type `T`. `T` must be a type
that implements [`serde::Deserialize`].

```
use argan::data::json::Json;
use serde::Deserialize;

#[derive(Deserialize)]
struct Person {
  first_name: String,
  last_name: String,
  age: u8,
}

async fn add_person(Json(person): Json<Person>) {
  // ...
}
```

By default, `Json` limits the body size to 2MiB. The body size limit can be changed by
specifying the SIZE_LIMIT const type parameter.

```
use argan::data::json::Json;
use serde::Deserialize;

#[derive(Deserialize)]
struct SurveyData {
  // ...
}

async fn save_survey_data(Json(survey_data): Json<SurveyData, { 512 * 1024 }>) {
  // ...
}
```
