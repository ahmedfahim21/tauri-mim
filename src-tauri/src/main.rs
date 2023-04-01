// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// Learn more about Tauri commands at https://tauri.app/v1/guides/features/command
use std::{net::SocketAddr, path::PathBuf, str::FromStr, sync::Arc, time::Duration};

use anyhow::Context;
use libmim::{
    http_api::{ApiAddTorrentResponse, HttpApi},
    http_api_client,
    peer_connection::PeerConnectionOptions,
    session::{
        AddTorrentOptions, AddTorrentResponse, ListOnlyResponse, ManagedTorrentState, Session,
        SessionOptions,
    },
    spawn_utils::{spawn, BlockingSpawner},
};
use log::{error, info, warn};
use size_format::SizeFormatterBinary as SF;



struct Opts {

    http_api_listen_addr: SocketAddr,

    single_thread_runtime: bool,

    disable_dht: bool,

    disable_dht_persistence: bool,

    worker_threads: Option<usize>,

    subcommand: SubCommand,

}

struct ServerStartOptions {
    /// The output folder to write to. If not exists, it will be created.
    output_folder: String,
}

struct ServerOpts {
    subcommand: ServerSubcommand,
}

enum ServerSubcommand {
    Start(ServerStartOptions),
}

struct DownloadOpts {
    
    torrent_path: Vec<String>,

    output_folder: Option<String>,

    sub_folder: Option<String>,

    only_files_matching_regex: Option<String>,

    list: bool,

    overwrite: bool,
}

// server start
// download [--connect-to-existing] --output-folder(required) [file1] [file2]

enum SubCommand {
    Server(ServerOpts),
    Download(DownloadOpts),
}

#[tauri::command]
fn download_fun(sent_url: &str) -> anyhow::Result<()> {
    let opts = Opts{http-api-listen-addr:"127.0.0.1:3030", worker_threads:8, 
    subcommand:SubCommand::Server(ServerOpts{subcommand:ServerSubcommand::Start(ServerStartOptions{output_folder:"./target".to_string})}), 
    subcommand:SubCommand::Download(DownloadOpts{torrent_path:sent_url})};
    

    let (mut rt_builder, spawner) = match opts.single_thread_runtime {
        true => (
            tokio::runtime::Builder::new_current_thread(),
            BlockingSpawner::new(false),
        ),
        false => (
            {
                let mut b = tokio::runtime::Builder::new_multi_thread();
                if let Some(e) = opts.worker_threads {
                    b.worker_threads(e);
                }
                b
            },
            BlockingSpawner::new(true),
        ),
    };

    let rt = rt_builder
        .enable_time()
        .enable_io()
        .max_blocking_threads(8)
        .build()?;

    rt.block_on(async_main(opts, spawner))
}

async fn async_main(opts: Opts, spawner: BlockingSpawner) -> anyhow::Result<()> {
    let sopts = SessionOptions {
        disable_dht: opts.disable_dht,
        disable_dht_persistence: opts.disable_dht_persistence,
        dht_config: None,
        peer_id: None,
        peer_opts: Some(PeerConnectionOptions {
            connect_timeout: opts.peer_connect_timeout.map(|d| d.0),
            read_write_timeout: opts.peer_read_write_timeout.map(|d| d.0),
            ..Default::default()
        }),
    };

    let stats_printer = |session: Arc<Session>| async move {
        loop {
            session.with_torrents(|torrents| {
                    for (idx, torrent) in torrents.iter().enumerate() {
                        match &torrent.state {
                            ManagedTorrentState::Initializing => {
                                info!("[{}] initializing", idx);
                            },
                            ManagedTorrentState::Running(handle) => {
                                let peer_stats = handle.torrent_state().peer_stats_snapshot();
                                let stats = handle.torrent_state().stats_snapshot();
                                let speed = handle.speed_estimator();
                                let total = stats.total_bytes;
                                let progress = stats.total_bytes - stats.remaining_bytes;
                                let downloaded_pct = if stats.remaining_bytes == 0 {
                                    100f64
                                } else {
                                    (progress as f64 / total as f64) * 100f64
                                };
                                info!(
                                    "[{}]: {:.2}% ({:.2}), down speed {:.2} MiB/s, fetched {}, remaining {:.2} of {:.2}, uploaded {:.2}, peers: {{live: {}, connecting: {}, queued: {}, seen: {}}}",
                                    idx,
                                    downloaded_pct,
                                    SF::new(progress),
                                    speed.download_mbps(),
                                    SF::new(stats.fetched_bytes),
                                    SF::new(stats.remaining_bytes),
                                    SF::new(total),
                                    SF::new(stats.uploaded_bytes),
                                    peer_stats.live,
                                    peer_stats.connecting,
                                    peer_stats.queued,
                                    peer_stats.seen,
                                );
                            },
                        }
                    }
                });
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    };

    match &opts.subcommand {
        SubCommand::Server(server_opts) => match &server_opts.subcommand {
            ServerSubcommand::Start(start_opts) => {
                let session = Arc::new(
                    Session::new_with_opts(
                        PathBuf::from(&start_opts.output_folder),
                        spawner,
                        sopts,
                    )
                    .await
                    .context("error initializing mim session")?,
                );
                spawn("Stats printer", stats_printer(session.clone()));
                let http_api = HttpApi::new(session);
                let http_api_listen_addr = opts.http_api_listen_addr;
                http_api.make_http_api_and_run(http_api_listen_addr).await
            }
        },
        SubCommand::Download(download_opts) => {
            if download_opts.torrent_path.is_empty() {
                anyhow::bail!("you must provide at least one URL to download")
            }
            let http_api_url = format!("http://{}", opts.http_api_listen_addr);
            let client = http_api_client::HttpApiClient::new(&http_api_url)?;
            let torrent_opts = AddTorrentOptions {
                only_files_regex: download_opts.only_files_matching_regex.clone(),
                overwrite: download_opts.overwrite,
                list_only: download_opts.list,
                force_tracker_interval: opts.force_tracker_interval.map(|d| d.0),
                output_folder: download_opts.output_folder.clone(),
                sub_folder: download_opts.sub_folder.clone(),
                ..Default::default()
            };
            let connect_to_existing = match client.validate_mim_server().await {
                Ok(_) => {
                    info!("Connected to HTTP API at {}, will call it instead of downloading within this process", client.base_url());
                    true
                }
                Err(err) => {
                    warn!("Error checking HTTP API at {}: {:}", client.base_url(), err);
                    false
                }
            };
            if connect_to_existing {
                for torrent_url in &download_opts.torrent_path {
                    match client
                        .add_torrent(torrent_url, Some(torrent_opts.clone()))
                        .await
                    {
                        Ok(ApiAddTorrentResponse { id, details }) => {
                            if let Some(id) = id {
                                info!("{} added to the server with index {}. Query {}/torrents/{}/(stats/haves) for details", details.info_hash, id, http_api_url, id)
                            }
                            for file in details.files {
                                info!(
                                    "file {:?}, size {}{}",
                                    file.name,
                                    SF::new(file.length),
                                    if file.included { "" } else { ", will skip" }
                                )
                            }
                        }
                        Err(err) => warn!("error adding {}: {:?}", torrent_url, err),
                    }
                }
                Ok(())
            } else {
                let session = Arc::new(
                    Session::new_with_opts(
                        download_opts
                            .output_folder
                            .as_ref()
                            .map(PathBuf::from)
                            .context(
                                "output_folder is required if can't connect to an existing server",
                            )?,
                        spawner,
                        sopts,
                    )
                    .await
                    .context("error initializing mim session")?,
                );
                spawn("Stats printer", stats_printer(session.clone()));
                let http_api = HttpApi::new(session.clone());
                let http_api_listen_addr = opts.http_api_listen_addr;
                spawn(
                    "HTTP API",
                    http_api.clone().make_http_api_and_run(http_api_listen_addr),
                );

                let mut added = false;

                for path in &download_opts.torrent_path {
                    let handle = match session.add_torrent(path, Some(torrent_opts.clone())).await {
                        Ok(v) => match v {
                            AddTorrentResponse::AlreadyManaged(handle) => {
                                info!(
                                    "torrent {:?} is already managed, downloaded to {:?}",
                                    handle.info_hash, handle.output_folder
                                );
                                continue;
                            }
                            AddTorrentResponse::ListOnly(ListOnlyResponse {
                                info_hash: _,
                                info,
                                only_files,
                            }) => {
                                for (idx, (filename, len)) in
                                    info.iter_filenames_and_lengths()?.enumerate()
                                {
                                    let included = match &only_files {
                                        Some(files) => files.contains(&idx),
                                        None => true,
                                    };
                                    info!(
                                        "File {}, size {}{}",
                                        filename.to_string()?,
                                        SF::new(len),
                                        if included { "" } else { ", will skip" }
                                    )
                                }
                                continue;
                            }
                            AddTorrentResponse::Added(handle) => {
                                added = true;
                                handle
                            }
                        },
                        Err(err) => {
                            error!("error adding {:?}: {:?}", &path, err);
                            continue;
                        }
                    };

                    http_api.add_mgr(handle.clone());
                }

                if download_opts.list {
                    Ok(())
                } else if added {
                    loop {
                        tokio::time::sleep(Duration::from_secs(60)).await;
                    }
                } else {
                    anyhow::bail!("no torrents were added")
                }
            }
        }
    }
}


fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![download_fun])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
