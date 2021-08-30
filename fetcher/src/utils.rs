use crate::{Fetcher, ReturnAction};
use reqwest;
use std::time::Duration;

const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/92.0.4515.131 Safari/537.36";
const MUSIC_FIELDS: &str = "fields=videoId,title,author,lengthSeconds";
const ITEM_PER_PAGE: usize = 10;
const REGION: &str = "region=NP";

impl super::ExtendDuration for Duration {
    fn to_string(self) -> String {
        let seconds: u64 = self.as_secs();
        let mut res = format!(
            "{minutes}:{seconds}",
            minutes = seconds / 60,
            seconds = seconds % 60
        );
        res.shrink_to_fit();
        res
    }

    // This function assumes that the string is alwayd formatted in "min:secs"
    fn from_string(inp: &str) -> Duration {
        let splitted = inp.split_once(':').unwrap();
        let total_secs: u64 = (60 * splitted.0.trim().parse::<u64>().unwrap_or_default())
            + splitted.1.trim().parse::<u64>().unwrap_or_default();
        Duration::from_secs(total_secs)
    }
}

impl Fetcher {
    pub fn new() -> Self {
        super::Fetcher {
            trending_now: None,
            search_res: (
                String::new(),
                super::SearchRes {
                    music: (Vec::new(), 0),
                    playlist: (Vec::new(), 0),
                    artist: (Vec::new(), 0),
                },
            ),
            servers: [
                "https://invidious.snopyta.org/api/v1",
                "https://vid.puffyan.us/api/v1",
                "https://ytprivate.com/api/v1",
                "https://ytb.trom.tf/api/v1",
                "https://invidious.namazso.eu/api/v1",
                "https://invidious.hub.ne.kr/api/v1",
            ],
            client: reqwest::ClientBuilder::default()
                .user_agent(USER_AGENT)
                .gzip(true)
                .build()
                .unwrap(),
            active_server_index: 0,
        }
    }
    pub fn change_server(&mut self) {
        self.active_server_index = (self.active_server_index + 1) % self.servers.len();
    }
}

impl Fetcher {
    async fn send_request<'de, Res>(
        &mut self,
        path: &str,
        retry_for: i32,
    ) -> Result<Res, ReturnAction>
    where
        Res: serde::de::DeserializeOwned,
    {
        let res = self
            .client
            .get(self.servers[self.active_server_index].to_string() + path)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await;

        match res {
            Ok(response) => {
                if let Ok(obj) = response.json::<Res>().await {
                    Ok(obj)
                } else {
                    Err(ReturnAction::Failed)
                }
            }
            Err(_) if retry_for > 0 => {
                self.change_server();
                Err(ReturnAction::Retry)
            }
            Err(_) => Err(ReturnAction::Failed),
        }
    }

    pub async fn get_trending_music(
        &mut self,
        page: usize,
    ) -> Result<&[super::MusicUnit], ReturnAction> {
        if self.trending_now.is_none() {
            let suffix = format!(
                "/trending?type=Music&{region}&{music_field}",
                region = REGION,
                music_field = MUSIC_FIELDS
            );

            let obj = self.send_request::<Vec<super::MusicUnit>>(&suffix, 2).await;
            match obj {
                Ok(mut res) => {
                    res.shrink_to_fit();
                    self.trending_now = Some(res);
                }
                Err(e) => return Err(e),
            }
        }

        let trending_now = self.trending_now.as_ref().unwrap();
        let lower_limit = ITEM_PER_PAGE * page;
        let upper_limit = std::cmp::min(trending_now.len(), lower_limit + ITEM_PER_PAGE);
        if lower_limit >= upper_limit {
            return Err(ReturnAction::EOR);
        }
        Ok(&trending_now[lower_limit..upper_limit])
    }

    pub async fn search_music<'me, 'inp>(
        &'me mut self,
        query: &'inp str,
        page: usize,
    ) -> Result<Vec<super::MusicUnit>, ReturnAction> {
        let mut prev_res = Vec::new();
        // Query string is same. So it is probably the request for next page
        if query == &self.search_res.0 {
            let lower_limit = ITEM_PER_PAGE * page as usize;
            let upper_limit =
                std::cmp::min(self.search_res.1.music.0.len(), lower_limit + ITEM_PER_PAGE);

            // Before reassigning the search result get the unserved search result from previous fetch
            if upper_limit > lower_limit {
                prev_res = self.search_res.1.music.0[lower_limit..upper_limit].to_vec();
            }
        }

        if prev_res.len() < ITEM_PER_PAGE {
            self.search_res.1.music.1 += 1;
            let suffix = format!(
                "/search?q={query}&type=video&{region}&page={page}&{fields}",
                query = query,
                region = REGION,
                fields = MUSIC_FIELDS,
                page = self.search_res.1.music.1
            );
            let obj = self.send_request::<Vec<super::MusicUnit>>(&suffix, 1).await;
            match obj {
                Ok(res) => {
                    self.search_res.1.music.0 = res;
                    let upper_limit = std::cmp::min(
                        self.search_res.1.music.0.len(),
                        ITEM_PER_PAGE - prev_res.len(),
                    );
                    prev_res.extend_from_slice(&self.search_res.1.music.0[0..upper_limit]);
                    if prev_res.is_empty() {
                        return Err(ReturnAction::EOR);
                    }
                }
                Err(e) => return Err(e),
            };
        }

        Ok(prev_res)
    }
}

// ------------- TEST ----------------
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn test_trending_extractor() {
        let mut fetcher = Fetcher::new();
        let mut page = 0;

        while let Ok(data) = fetcher.get_trending_music(page).await {
            println!("--------- Trending [{}] ----------", page);
            println!("{:#?}", data);
            page += 1;
        }
    }

    #[tokio::test]
    async fn check_format() {
        let sample_response = r#"{
                                    "title": "Some song title",
                                    "videoId": "WNgO6G7uERU",
                                    "author": "CHHEWANG",
                                    "lengthSeconds": 271
                                }"#;
        let obj: super::super::MusicUnit = serde_json::from_str(sample_response).unwrap();
        assert_eq!(
            obj,
            super::super::MusicUnit {
                liked: false,
                artist: "CHHEWANG".to_string(),
                name: "Some song title".to_string(),
                duration: "4:31".to_string(),
                path: "https://www.youtube.com/watch?v=WNgO6G7uERU".to_string(),
            },
        );
    }
    #[tokio::test]
    async fn check_music_search() {
        let mut fetcher = Fetcher::new();
        let obj = fetcher.search_music("Spotify chill&cool=mix", 1).await;
        eprintln!("{:#?}", obj);
    }
}
