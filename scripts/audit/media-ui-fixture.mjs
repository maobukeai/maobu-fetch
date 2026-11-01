// Isolated component fixture: synthetic metadata only, no native commands or accounts.
export const mediaUiFixture = `<!doctype html><html><head><meta charset="UTF-8"><link rel="icon" href="data:,"></head>
<body><div id="root"></div><script type="module">
import React from 'react';
import { createRoot } from 'react-dom/client';
import { NewTaskDialog } from '/src/components/modals/NewTaskDialog.tsx';
import { api } from '/src/api.ts';
import '/src/styles.css';
const params = new URLSearchParams(location.search);
const ffmpeg = params.get('ffmpeg') !== 'false';
const format = (id,height,video,audio,merge=false) => ({id,label:id,height,has_video:video,has_audio:audio,requires_ffmpeg:merge,extension:'mp4'});
const video = {title:'有声画质选择',media_type:'video',formats:[format('silent-4k',2160,true,false),{...format('direct-360',360,true,true),file_size:113054403,file_size_approximate:true},format('merged-1080',1080,true,true,true)],subtitles:[],ffmpeg_available:ffmpeg};
const collection = {title:'真实分集URL'.repeat(12),media_type:'collection',formats:[],subtitles:[],ffmpeg_available:ffmpeg,extractor:'BiliBili',episodes:[
 {index:1,title:'第一集'+ '长标题'.repeat(30),url:'https://www.bilibili.com/video/BV1TEST?p=1',duration:42},
 {index:2,title:'第二集',url:'https://www.bilibili.com/video/BV1TEST?p=2',duration:60}]};
const original = {title:'actual-source.mp4',extractor:'pikpak',media_type:'video',formats:[{id:'original',label:'原文件（音视频属性待检测）',original_file:true,has_video:false,has_audio:false,requires_ffmpeg:false,extension:'mp4',url:'https://cdn.example.test/actual-source.mp4'}],subtitles:[],ffmpeg_available:false};
const unknown = {...video,title:'未确认音轨原文件',formats:[{...format('unknown-4k',2160,true,false),audio_unknown:true}]};
const audit = window.audit = {calls:[],batchCalls:[],probeOptions:[],probeUrls:[],result:params.get('mode') === 'collection' ? collection : params.get('mode') === 'original' ? original : params.get('mode') === 'unknown' ? unknown : video};
for (const name of Object.keys(api)) api[name] = async () => [];
api.toolStatus = async () => ({state:'ready',yt_dlp_available:true,ffmpeg_available:ffmpeg});
api.subscribeMediaTools = async () => () => {};
api.categoryRuleApply = async () => null;
api.taskTemplateTest = async () => ({matched:false});
api.mediaDetectPlatform = async () => params.get('pendingPlatform') === '1' ? new Promise(() => {}) : 'bilibili';
api.mediaNormalizeUrl = async input => input;
api.platformCompatibilityGet = async () => null;
api.mediaCredentialGet = async () => null;
api.precheck = async () => {throw new Error('隔离审查未调用网络预检');};
api.probeMedia = async (url,options) => {audit.probeUrls.push(url);audit.probeOptions.push(options);return audit.result;};
api.add = async request => {audit.calls.push(request);return {id:'fixture-'+audit.calls.length,...request};};
api.addBatch = async (urls,request) => {audit.batchCalls.push({urls,request});return [];};
const settings = {download_dir:'C:/AuditOnly',connections_per_download:4,default_collision_policy:'rename',default_completion_action:'none'};
createRoot(document.getElementById('root')).render(React.createElement(NewTaskDialog,{settings,allTasks:[],defaultUrl:'https://www.bilibili.com/video/BV1TEST',onClose:()=>{},onCreated:tasks=>{audit.created=tasks;}}));
</script></body></html>`;
