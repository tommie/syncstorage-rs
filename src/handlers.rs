//! API Handlers
use actix_web::{
    Error, FromRequest, FutureResponse, HttpRequest, HttpResponse, Json, Path, Query, State,
};
use futures::future::result;
// Hawk lib brings in some libs that don't compile at the moment for some reason
//use hawk::
use serde::de::{Deserialize, Deserializer};

use data;

/// This is the global HTTP state object that will be made available to all
/// HTTP API calls.
pub struct ServerState;

#[derive(Debug, Deserialize)]
struct HawkHeader(String);

/// Extract a HAWK header
impl<S> FromRequest<S> for HawkHeader {
    type Config = ();
    type Result = Result<HawkHeader, Error>;

    fn from_request(req: &HttpRequest<S>, _cfg: &Self::Config) -> Self::Result {
        // TODO: Actually extract the Hawk Header
        // There are a couple of layers of signing involved here, that eventually
        // chain back to a secret key shared between this storage node and the tokenserver.
        // The Authorization header will look like this:
        //    Authorization: Hawk id="<...>", ts="1353832234", nonce="j4h3g2", a", mac="6R4rV5iE+NPoym+WwjeHzjAGXUtLNIxmo1vpMofpLAE="
        // Where the "id" field is a signed-and-base64-encoded JSON blob of user metadata,
        // produced by the python "tokenlib" library (https://github.com/mozilla-services/tokenlib)
        // The decoding procedure must proceed as follows:
        //
        //   * Obtain the `master_secret` master token secret from config
        //   * Derive `signing_secret = HKDF-SHA256(master_secret, size=32, salt=None, info="services.mozilla.com/tokenlib/v1/signing")`
        //   * Extract the `id` from the Hawk auth header
        //   * urlsafe_b64decode `id` and split off the last 32 bytes to give (`payload`, `signature`)
        //   * Calculate `HMAC-SHA256(payload, signing_secret)` and check that it matches `signature`
        //   * JSON decode `payload` to give an object like: {
        //       'userid': 42,
        //       'expires': 1329875384.073159
        //       'salt': '1c033f'
        //     }
        //   * Check that the "expires" timestamp is not in the past.
        //   * Derive `token_secret = HKDF-SHA256(master_secret, size=32, salt=payload["salt"], info="services.mozilla.com/tokenlib/v1/derive/" + id)`
        //   * Use `token_secret` as the secret key for calculating the Hawk request MAC
        //   * Check that the Hawk request MAC matches the "mac" value from the Hawk authorization header.
        //   * Use the `userid` and other user meta-data from the decoded `payload`.
        //
        // Phew!  That's a lot of steps, but they all exist in order to help ensure that tokens are only
        // used by the right user, on the right storage node.  We should probably create our own local
        // rust port of https://github.com/mozilla-services/tokenlib to encapsulate those details.
        Ok(HawkHeader("token".to_string()))
    }
}

macro_rules! endpoint {
    ($handler:ident: $data:ident ($path:ident: $path_type:ty $(, $param:ident: $type:ty)*) {$($property:ident: $value:expr,)*}) => {
        pub fn $handler(
            ($path, _state$(, $param)*): (Path<$path_type>, State<ServerState>$(, $type)*),
        ) -> FutureResponse<HttpResponse> {
            let _data = data::$data {
                $($property: $value,)*
            };
            Box::new(result(Ok(HttpResponse::Ok().json(()))))
        }
    }
}

macro_rules! info_endpoints {
    ($($handler:ident: $data:ident,)+) => ($(
        endpoint! {
            $handler: $data (params: UidParam) {
                user_id: params.uid.clone(),
            }
        }
    )+)
}

info_endpoints! {
    collections: Collections,
    collection_counts: CollectionCounts,
    collection_usage: CollectionUsage,
    configuration: Configuration,
    quota: Quota,
}

#[derive(Deserialize)]
pub struct UidParam {
    uid: String,
}

macro_rules! collection_endpoints {
    ($($handler:ident: $data:ident ($($param:ident: $type:ty),*) {$($property:ident: $value:expr,)*},)+) => ($(
        endpoint! {
            $handler: $data (params: CollectionParams $(, $param: $type)*) {
                user_id: params.uid.clone(),
                collection: params.collection.clone(),
                $($property: $value,)*
            }
        }
    )+)
}

collection_endpoints! {
    delete_collection: DeleteCollection (query: Query<DeleteCollectionQuery>) {
        bso_ids: query.ids.as_ref().map_or_else(|| Vec::new(), |ids| ids.0.clone()),
    },
    get_collection: GetCollection () {},
    post_collection: PostCollection (body: Json<Vec<PostCollectionBody>>) {
        bsos: body.into_inner().into_iter().map(From::from).collect(),
    },
}

#[derive(Deserialize)]
pub struct DeleteCollectionQuery {
    ids: Option<BsoIds>,
}

pub struct BsoIds(pub Vec<String>);

impl<'d> Deserialize<'d> for BsoIds {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'d>,
    {
        let value: String = Deserialize::deserialize(deserializer)?;
        Ok(BsoIds(value.split(",").map(|id| id.to_string()).collect()))
    }
}

#[derive(Deserialize, Serialize)]
pub struct PostCollectionBody {
    pub id: String,
    pub sortindex: Option<i64>,
    pub payload: Option<String>,
    pub ttl: Option<i64>,
}

impl From<PostCollectionBody> for data::PostCollectionBso {
    fn from(body: PostCollectionBody) -> data::PostCollectionBso {
        data::PostCollectionBso {
            bso_id: body.id.clone(),
            sortindex: body.sortindex,
            payload: body.payload.as_ref().map(|payload| payload.clone()),
            ttl: body.ttl,
        }
    }
}

#[derive(Deserialize)]
pub struct CollectionParams {
    uid: String,
    collection: String,
}

macro_rules! bso_endpoints {
    ($($handler:ident: $data:ident ($($param:ident: $type:ty),*) {$($property:ident: $value:expr,)*},)+) => ($(
        endpoint! {
            $handler: $data (params: BsoParams $(, $param: $type)*) {
                user_id: params.uid.clone(),
                collection: params.collection.clone(),
                bso_id: params.bso.clone(),
                $($property: $value,)*
            }
        }
    )+)
}

bso_endpoints! {
    delete_bso: DeleteBso () {},
    get_bso: GetBso () {},
    put_bso: PutBso (body: Json<BsoBody>) {
        sortindex: body.sortindex,
        payload: body.payload.as_ref().map(|payload| payload.clone()),
        ttl: body.ttl,
    },
}

#[derive(Deserialize)]
pub struct BsoParams {
    uid: String,
    collection: String,
    bso: String,
}

#[derive(Deserialize, Serialize)]
pub struct BsoBody {
    pub sortindex: Option<i64>,
    pub payload: Option<String>,
    pub ttl: Option<i64>,
}
