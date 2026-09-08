//! Reverse-proxy the iDRAC HTML5 / eHTML5 virtual console from TCS's origin.
//!
//! The operator's browser never talks to the BMC: TCS fetches viewer assets
//! with the session cookie jar, strips frame-busting headers, rewrites
//! absolute BMC URLs onto the session prefix, and injects a small script that
//! patches `fetch` / XHR / `WebSocket` / `Worker` so runtime requests stay
//! under the TCS path. WebSocket upgrades are bridged transparently (the
//! viewer itself speaks VNC-over-WS or Avocent).
//!
//! A dedicated `{prefix}/__rfb/{port}` WebSocket maps viewers that still dial
//! the remote-presence port (5900/5901) onto a TLS (then plain) upstream.

use axum::body::Body;
use axum::extract::ws::{Message, WebSocket};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::Response;
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::connect_async_tls_with_config;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message as TwsMsg;

use super::idrac::{gui_client, IdracSession, CHROME_UA};
use super::session::bmc_origin;
use super::tls::insecure_connector;
use crate::AppError;

const MAX_BODY: usize = 16 * 1024 * 1024;

/// Paths the iDRAC viewer commonly requests with a leading slash. Rewritten
/// onto the TCS session prefix in HTML/JS/CSS so parser-loaded assets resolve.
const ABS_PREFIXES: &[&str] = &[
    "/console",
    "/html5.html",
    "/login.html",
    "/login",
    "/restgui",
    "/sysmgmt",
    "/redfish",
    "/public",
    "/Applications",
    "/ws",
    "/data",
    "/cgi-bin",
    "/session",
    "/vconsole",
    "/viewer",
    "/eHTML5",
    "/VConsole",
    "/kvm",
    "/wsman",
    "/help",
    "/images",
    "/css",
    "/js",
    "/lib",
    "/app",
    "/locale",
    "/resources",
    "/favicon",
];

pub fn is_safe_idrac_path(path: &str) -> bool {
    let t = path.trim();
    if t.is_empty() {
        return true;
    }
    if t.contains('\\') || t.split('/').any(|seg| seg == "..") {
        return false;
    }
    // `__rfb/5900` is our synthetic KVM-port relay. Avocent appends `/`.
    if let Some(rest) = t.strip_prefix("__rfb/") {
        let rest = rest.trim_end_matches('/');
        return !rest.is_empty() && rest.len() <= 5 && rest.chars().all(|c| c.is_ascii_digit());
    }
    t.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/' | '$' | '~' | '+'))
}

/// Patch `WebSocket` in both the page and Worker scripts. Avocent's HTML5
/// viewer dials `wss://<ip>:5900` from `websockethandler.js` (a Worker),
/// which does not see the page-level constructor wrap.
fn websocket_hook_js(prefix: &str) -> String {
    let pfx = prefix.trim_end_matches('/');
    format!(
        r#"(function(){{try{{var P="{pfx}";var g=typeof self!=="undefined"?self:this;if(!g||!g.WebSocket)return;if(g.__tcsWS&&g.__tcsWS.__tcsRfb)return;var OW=g.WebSocket;function wrap(u){{try{{var s=String(u);var x=new URL(s,"https://dummy.invalid/");var port=x.port||"";if(!port){{var m=s.match(/:590([0-2])(?:[^\d]|$)/);if(m)port="590"+m[1];}}if(port==="5900"||port==="5901"||port==="5902"){{var host=x.hostname||(g.location&&g.location.hostname)||"";if(!host)return u;var y=new URL("https://"+host+P+"/__rfb/"+port);y.protocol="wss:";return y.toString();}}}}catch(e){{}}return u;}}function W(u,p){{u=wrap(u);return p!==undefined?new OW(u,p):new OW(u);}}W.prototype=OW.prototype;W.CONNECTING=OW.CONNECTING;W.OPEN=OW.OPEN;W.CLOSING=OW.CLOSING;W.CLOSED=OW.CLOSED;W.__tcsRfb=1;g.__tcsWS=W;try{{g.WebSocket=W;}}catch(e){{}}try{{Object.defineProperty(g,"WebSocket",{{configurable:true,writable:true,value:W}});}}catch(e){{}}}}catch(e){{}}}})();"#
    )
}

/// Build the injected rewrite script + `<base>` for a viewer HTML document.
pub fn inject_prefix_hooks(html: &str, prefix: &str) -> String {
    let pfx = prefix.trim_end_matches('/');
    let ws_hook = websocket_hook_js(pfx);
    // iDRAC 7 child frames use `top.TOKEN_VALUE` / `top.snb` / `top.treelist`
    // expecting index.html to be the browsing-context top. Inside TCS it is
    // not, so we expose `tcsTop` as the nearest window still on this session.
    let script = format!(
        r#"<script>{ws_hook}(function(){{var P="{pfx}";function tcsIsFs(w){{try{{return !!(w&&w.document&&w.document.getElementsByTagName("frameset").length);}}catch(e){{return false;}}}}function tcsTopGet(){{var list=[];function add(x){{try{{if(x&&list.indexOf(x)<0)list.push(x);}}catch(e){{}}}}add(window);try{{var fe=window.frameElement;if(fe&&fe.ownerDocument)add(fe.ownerDocument.defaultView);}}catch(e){{}}var w=window;try{{while(w){{add(w);if(!w.parent||w.parent===w||w.parent===window.top)break;w=w.parent;}}}}catch(e){{}}function hasLoc(x){{try{{return !!(x&&x.localeObj&&x.localeObj["btn_ok"]);}}catch(e){{return false;}}}}for(var i=0;i<list.length;i++){{try{{if(tcsIsFs(list[i])&&hasLoc(list[i]))return list[i];}}catch(e){{}}}}for(var i=0;i<list.length;i++){{try{{if(list[i].TOKEN_VALUE&&hasLoc(list[i]))return list[i];}}catch(e){{}}}}for(var i=0;i<list.length;i++){{if(hasLoc(list[i]))return list[i];}}for(var i=0;i<list.length;i++){{if(tcsIsFs(list[i]))return list[i];}}return list[0]||window;}}function tcsEl(name){{var doc;try{{doc=tcsTopGet().document;}}catch(e){{return null;}}try{{var n=doc.getElementsByName(name);if(n&&n[0])return n[0];}}catch(e){{}}try{{var fs=doc.getElementsByTagName("frame");for(var i=0;i<fs.length;i++){{if(fs[i].getAttribute("name")===name)return fs[i];}}}}catch(e){{}}return null;}}function tcsWin(f){{if(!f)return null;try{{if(f.document)return f;}}catch(e){{}}try{{if(f.contentWindow)return f.contentWindow;}}catch(e){{}}return null;}}function tcsFrame(name){{try{{var w=tcsWin(window.parent&&window.parent.frames&&window.parent.frames[name]);if(w)return w;}}catch(e){{}}try{{var r=tcsTopGet();var w=tcsWin(r.frames&&r.frames[name]);if(w)return w;}}catch(e){{}}try{{var w=tcsWin(window.parent&&window.parent[name]);if(w)return w;}}catch(e){{}}return tcsWin(tcsEl(name));}}function tcsAbs(u){{if(typeof u!=="string"||!u)return u;if(u.charAt(0)==='#')return u;if(/^[a-zA-Z][a-zA-Z0-9+.-]*:/.test(u)){{try{{var x=new URL(u,location.href);if(x.origin===location.origin&&x.pathname.indexOf(P)!==0){{x.pathname=P+x.pathname;return x.toString();}}}}catch(e){{}}return u;}}if(u.indexOf("//")===0)return u;if(u.charAt(0)==="/")return u.indexOf(P)===0?u:P+u;return P+"/"+u.replace(/^\.\//,"");}}function tcsNav(name,u){{var abs=tcsAbs(u);if(typeof name!=="string"&&name&&name.name)name=name.name;if(!name)return;try{{var el=tcsEl(name);if(el){{el.src=abs;return;}}}}catch(e){{}}try{{tcsTopGet().postMessage({{tcsIdracNav:name,url:abs}},"*");}}catch(e){{}}}}function tcsCall(frame,fn){{var args=Array.prototype.slice.call(arguments,2);try{{var w=tcsFrame(frame);if(w&&typeof w[fn]==="function")return w[fn].apply(w,args);}}catch(e){{}}try{{tcsTopGet().postMessage({{tcsIdracCall:1,frame:frame,fn:fn,args:args}},"*");}}catch(e){{}}}}try{{window.addEventListener("message",function(ev){{var d=ev.data;if(!d||!tcsIsFs(window))return;if(d.tcsIdracNav){{try{{var el=tcsEl(d.tcsIdracNav);if(el)el.src=d.url;}}catch(e){{}}}}if(d.tcsIdracCall){{try{{var w=tcsFrame(d.frame);if(w&&typeof w[d.fn]==="function")w[d.fn].apply(w,d.args||[]);}}catch(e){{}}}}}});}}catch(e){{}}try{{Object.defineProperty(window,"tcsTop",{{get:tcsTopGet}});}}catch(e){{window.tcsTop=tcsTopGet();}}window.tcsAbs=tcsAbs;window.tcsNav=tcsNav;window.tcsFrame=tcsFrame;window.tcsCall=tcsCall;try{{window.bPreviewLoaded=true;}}catch(e){{}}function tcsOpenCat(cat,tab,id){{var list=null;try{{if(window.data)list=window.data;}}catch(e){{}}try{{if(!list){{var tl=tcsFrame("treelist");if(tl&&tl.data)list=tl.data;}}}}catch(e){{}}try{{if(!list&&tcsTopGet().__tcsTreeData)list=tcsTopGet().__tcsTreeData;}}catch(e){{}}if(!list||!cat)return;var url="";for(var i=0;i<list.length;i++){{var r=list[i];if(!r||String(r[0])!==String(cat)||!r[3])continue;if(id&&String(r[2])===String(id)){{url=r[3];break;}}if(!url)url=r[3];}}if(!url)return;url+=url.indexOf("?")<0?"?":"&";url+="cat="+encodeURIComponent(cat);if(tab)url+="&tab="+encodeURIComponent(tab);if(id)url+="&id="+encodeURIComponent(id);tcsNav("da",url);}}window.tcsOpenCat=tcsOpenCat;try{{setInterval(function(){{try{{if(window.data&&window.data.length)tcsTopGet().__tcsTreeData=window.data;}}catch(e){{}}}},1000);}}catch(e){{}}try{{if(tcsIsFs(window)&&location.href.indexOf("virtualconsolehtml5")<0){{var _n=0;var _iv=setInterval(function(){{_n++;try{{var T=tcsTopGet();if(T&&T.TOKEN_VALUE&&T.ssnObj&&T.ssnObj.aimSession!=null&&!window.__tcsKvmLaunched){{window.__tcsKvmLaunched=1;try{{sessionStorage.setItem("tcsIdracLocale",JSON.stringify(T.localeObj||{{}}));}}catch(e){{}}try{{sessionStorage.setItem("tcsIdracLang",String(T.lang||"en"));}}catch(e){{}}var port="5900";try{{if(T.aimGetIntPropObj&&T.aimGetIntPropObj["rp_int_apcp_port"])port=String(T.aimGetIntPropObj["rp_int_apcp_port"]);}}catch(e){{}}var u="virtualconsolehtml5.html?ipAddr="+encodeURIComponent(location.hostname)+"&kvmPort="+encodeURIComponent(port)+"&vmPriv=true&title=KVM&lang="+encodeURIComponent(String(T.lang||"en"))+"&aimSession="+encodeURIComponent(String(T.ssnObj.aimSession))+"&ST2="+encodeURIComponent(String(T.TOKEN_VALUE))+"&TokenName="+encodeURIComponent(String(T.TOKEN_NAME1||"ST1"))+"&TokenKey="+encodeURIComponent(String(T.TOKEN_VALUE1||""));clearInterval(_iv);window.location.replace(tcsAbs(u));}}}}catch(e){{}}if(_n>80)clearInterval(_iv);}},250);}}}}catch(e){{}}function f(u){{if(typeof u!=="string")return u;if(u.charAt(0)==="/"&&u.indexOf(P)!==0&&u.indexOf("//")!==0)return P+u;return u;}}try{{var OF=window.fetch;window.fetch=function(u,o){{if(typeof u==="string")u=f(u);else if(u&&u.url){{try{{u=new Request(f(u.url),u)}}catch(e){{}}}}return OF.call(this,u,o);}};var xo=XMLHttpRequest.prototype.open;XMLHttpRequest.prototype.open=function(m,u){{arguments[1]=f(u);return xo.apply(this,arguments);}};var OW=window.WebSocket;window.WebSocket=function(u,p){{try{{var x=new URL(u,location.href);x.protocol=location.protocol==="https:"?"wss:":"ws:";if(x.port==="5900"||x.port==="5901"||x.port==="5902"){{x.host=location.host;x.pathname=P+"/__rfb/"+x.port;}}else{{x.host=location.host;if(x.pathname.indexOf(P)!==0)x.pathname=P+x.pathname;}}u=x.toString();}}catch(e){{}}return p!==undefined?new OW(u,p):new OW(u);}};window.WebSocket.prototype=OW.prototype;window.WebSocket.CONNECTING=OW.CONNECTING;window.WebSocket.OPEN=OW.OPEN;window.WebSocket.CLOSING=OW.CLOSING;window.WebSocket.CLOSED=OW.CLOSED;if(window.Worker){{var Wr=window.Worker;window.Worker=function(u,o){{return new Wr(f(u),o);}};}}var sa=HTMLElement.prototype.setAttribute;HTMLElement.prototype.setAttribute=function(n,v){{if((n==="src"||n==="href"||n==="action")&&typeof v==="string")v=f(v);return sa.call(this,n,v);}};}}catch(e){{}}}})();</script>"#
    );
    let base = format!("<base href=\"{pfx}/\">");
    // Stock iDRAC 7 sysSummary writes #progressPage and only hides it for
    // OEM custom GUI. The overlay otherwise covers the summary iframe.
    let hide_progress = r#"<style id="tcsHideProgress">#progressPage,.progressBackground,.progressContainer,#progressGraphic,#progressScreen{display:none!important;visibility:hidden!important}html,body,#dataarea,#formArea,#contentArea,.data-area,.table_container,.container_content,table.system_summary{display:block!important;visibility:visible!important;color:#111!important}#contentArea,#formArea{min-height:240px;background:#fff}iframe#sysIframe{position:absolute!important;top:0;left:0;width:100%;height:100%}#canvasParentDiv,#kvmCanvas,canvas#kvmCanvas{display:block!important;width:100%;height:100%;min-height:360px;background:#111}</style>"#;
    let hook = format!("{script}{hide_progress}{base}");
    let lower = html.to_ascii_lowercase();
    if let Some(idx) = lower.find("<head") {
        if let Some(close) = html[idx..].find('>') {
            let pos = idx + close + 1;
            let mut out = String::with_capacity(html.len() + hook.len());
            out.push_str(&html[..pos]);
            out.push_str(&hook);
            out.push_str(&html[pos..]);
            return out;
        }
    }
    format!("{hook}{html}")
}

pub fn rewrite_absolute_urls(text: &str, prefix: &str, bmc_host: &str) -> String {
    let pfx = prefix.trim_end_matches('/');
    let origin = bmc_origin(bmc_host);
    let mut t = text.to_string();
    // Absolute BMC origin → session prefix (http and https, with/without port).
    for scheme in ["https://", "http://", "wss://", "ws://"] {
        let needle = format!("{scheme}{bmc_host}");
        t = t.replace(&needle, pfx);
        // host may be stored as host:port already; also try without default 443.
        if let Some((h, _)) = bmc_host.rsplit_once(':') {
            let n2 = format!("{scheme}{h}");
            t = t.replace(&n2, pfx);
        }
    }
    t = t.replace(&origin, pfx);

    for abs in ABS_PREFIXES {
        let from_dq = format!("\"{abs}");
        let to_dq = format!("\"{pfx}{abs}");
        t = t.replace(&from_dq, &to_dq);
        let from_sq = format!("'{abs}");
        let to_sq = format!("'{pfx}{abs}");
        t = t.replace(&from_sq, &to_sq);
    }
    let doubled = format!("{pfx}{pfx}");
    t = t.replace(&doubled, pfx);

    // HTML attributes: src="/foo" (not protocol-relative src="//")
    t = rewrite_attr_slash_urls(&t, pfx);
    t
}

/// iDRAC 7 `CheckTop()` does `top.document.location.href.search('index')` and
/// if missing sends the *top* window to `/start.html`. Inside TCS the top
/// window is the TCS app (no `index` in the URL), so a successful login is
/// immediately replaced by the iDRAC login page *and* the TCS tab itself
/// navigates away. Force the check to pass and rewrite remaining top-window
/// location writes into no-ops (the iframe is same-origin with TCS).
fn neutralize_idrac_framebust(text: &str) -> String {
    let t = text
        .replace(
            "if ( top.document.location.href.search('index') < 0 )",
            "if ( false )",
        )
        .replace("if (top.frames.length >= 1)", "if (false)")
        .replace("if(top.frames.length>=1)", "if(false)")
        .replace("if (top != self)", "if (false)")
        .replace("if (top !== self)", "if (false)")
        .replace("if(top!=self)", "if(false)")
        .replace(
            "var pluginType = top.aimGetIntPropObj['gui_kvm_plugin_type'];",
            "var pluginType = (typeof KVM_HTML_PLUGIN!==\"undefined\"?KVM_HTML_PLUGIN:\"2\");",
        );
    // sysSummary.html only calls progressBar.hide() when isCustomGui==1.
    // Real firmware uses tabs inside that block, so a literal \n replace misses.
    let hide = regex::Regex::new(
        r"if\s*\(\s*top\.isCustomGui\s*==\s*1\s*\)\s*\{\s*progressBar\.hide\(\);\s*\}",
    )
    .expect("idrac hideProgress");
    let t = hide
        .replace_all(
            &t,
            "progressBar.hide(); var _ps=document.getElementById(\"progressScreen\"); if(_ps)_ps.style.display=\"none\"; var _pp=document.getElementById(\"progressPage\"); if(_pp)_pp.style.display=\"none\";",
        )
        .into_owned();
    // virtualconsole.html: replace launchKVM so Safari/Firefox never take the
    // Java `window.open(jnlp, '_self')` path.
    let launch = regex::Regex::new(
        r"function\s+launchKVM\s*\(\s*arg\s*\)\s*\{[\s\S]*?launchConsole\s*\(\s*isKVMEnabled\s*,\s*arg\s*\)\s*;\s*\}",
    )
    .expect("idrac launchKVM");
    let t = launch
        .replace_all(
            &t,
            r#"function launchKVM(arg){try{var T=(typeof tcsTop!=="undefined"&&tcsTop)?tcsTop:window;var port="5900";try{var el=document.getElementById("kvmPort");if(el&&el.value)port=el.value;}catch(e){}try{if(T.aimGetIntPropObj&&T.aimGetIntPropObj["rp_int_apcp_port"])port=String(T.aimGetIntPropObj["rp_int_apcp_port"]);}catch(e){}var aim="",st2="",tokName="ST1",tokKey="",lang="en";try{if(T.ssnObj&&T.ssnObj["aimSession"]!=null)aim=String(T.ssnObj["aimSession"]);}catch(e){}try{if(T.TOKEN_VALUE)st2=String(T.TOKEN_VALUE);}catch(e){}try{if(T.TOKEN_NAME1)tokName=String(T.TOKEN_NAME1);}catch(e){}try{if(T.TOKEN_VALUE1)tokKey=String(T.TOKEN_VALUE1);}catch(e){}try{if(T.lang)lang=String(T.lang);}catch(e){}var u="virtualconsolehtml5.html?ipAddr="+encodeURIComponent(location.hostname)+"&kvmPort="+encodeURIComponent(port)+"&vmPriv=true&title=KVM&lang="+encodeURIComponent(lang)+"&aimSession="+encodeURIComponent(aim)+"&ST2="+encodeURIComponent(st2)+"&TokenName="+encodeURIComponent(tokName)+"&TokenKey="+encodeURIComponent(tokKey);if(typeof tcsNav==="function")tcsNav("da",u);else window.location.href=u;}catch(e){}}"#,
        )
        .into_owned();
    // sysSummaryData.html top-level `top.aimGetBoolPropObj['x']` throws if
    // the frameset has not finished session/locale yet (da starts early).
    let t = t
        .replace("top.aimGetBoolPropObj[", "(tcsTop.aimGetBoolPropObj||{})[")
        .replace("top.aimGetIntPropObj[", "(tcsTop.aimGetIntPropObj||{})[")
        .replace("top.aimGetPropObj[", "(tcsTop.aimGetPropObj||{})[")
        .replace("top.localeObj[", "(tcsTop.localeObj||{})[");
    // `top.location = "/start.html"` / `.replace(...)` would still leave TCS.
    // Turn those into a throwaway assignment / void call. No lookahead — the
    // `regex` crate rejects it.
    let assign = regex::Regex::new(
        r#"(?i)\b(?:window\s*\.\s*)?(?:top|parent)\s*\.\s*(?:document\s*\.\s*)?location\s*(?:\.\s*href\s*)?\s*="#,
    )
    .expect("idrac top-nav assign");
    let t = assign.replace_all(&t, "tcsNoop =").into_owned();
    let replace_call = regex::Regex::new(
        r#"(?i)\b(?:window\s*\.\s*)?(?:top|parent)\s*\.\s*(?:document\s*\.\s*)?location\s*\.\s*(?:replace|assign)\s*\("#,
    )
    .expect("idrac top-nav replace");
    let t = replace_call.replace_all(&t, "void (").into_owned();
    // Exact iDRAC 7 assignment: lsnb.html does `parent.da.location = link`
    // which otherwise resolves `sysSummary.html` against the TCS origin (404).
    let t = t.replace("parent.da.location = link", r#"tcsNav("da", link)"#);
    let t = t.replace(
        "parent.snb.f_getHTML(newCat, tab, id, flag);",
        r#"tcsCall("snb","f_getHTML",newCat,tab,id,flag);tcsOpenCat(newCat,tab,id);"#,
    );
    let t = t.replace(
        "top.snb.f_getHTML(cat, tab, id, flag);",
        r#"tcsCall("snb","f_getHTML",cat,tab,id,flag);tcsOpenCat(cat,tab,id);"#,
    );
    let t = t.replace(
        r#"parent.snb.f_getHTML_F(tab, "", "", "");"#,
        r#"tcsCall("snb","f_getHTML_F",tab,"","","");tcsOpenCat(tab,"","");"#,
    );
    // iDRAC 7 Launch Console: load HTML5 viewer in the da frame. Firmware
    // indents this assignment with tabs, so a literal replace misses.
    let open_html = regex::Regex::new(
        r"top\s*\.\s*htmlViewerWindow\s*=\s*window\.open\s*\(\s*htmlURL\s*,[^;]*\)",
    )
    .expect("idrac htmlURL open");
    let t = open_html
        .replace_all(
            &t,
            r#"try{tcsNav("da", htmlURL);}catch(e){} try{tcsTop.htmlViewerWindow=window;}catch(e){}"#,
        )
        .into_owned();
    // HTML5 viewer is a popup in stock firmware (`window.opener.top`). In the
    // da frame there is no opener, and `tcsTop` must be the frameset — not this
    // page's empty `localeObj`.
    let t = t.replace(
        "localeObj1 = window.opener.top.localeObj;",
        r#"localeObj1=(function(){try{var s=sessionStorage.getItem("tcsIdracLocale");if(s){var o=JSON.parse(s);if(o&&o["viewer_connectingViewer"])return o;}}catch(e){}try{var t=tcsTop;if(t&&t.localeObj&&t.localeObj["viewer_connectingViewer"])return t.localeObj;}catch(e){}try{var w=window.parent;while(w){if(w.localeObj&&w.localeObj["viewer_connectingViewer"])return w.localeObj;if(!w.parent||w.parent===w)break;w=w.parent;}}catch(e){}return {};})();"#,
    );
    let t = t.replace(
        "localeObj = window.opener.top.localeObj;",
        r#"localeObj=(function(){try{var s=sessionStorage.getItem("tcsIdracLocale");if(s){var o=JSON.parse(s);if(o&&o["btn_ok"])return o;}}catch(e){}try{var t=tcsTop;if(t&&t.localeObj&&t.localeObj["btn_ok"])return t.localeObj;}catch(e){}try{var w=window.parent;while(w){if(w.localeObj&&w.localeObj["btn_ok"])return w.localeObj;if(!w.parent||w.parent===w)break;w=w.parent;}}catch(e){}return {};})();"#,
    );
    let t = t.replace(
        "window.opener.top.lang.toLowerCase()",
        r#"(String((function(){try{var s=sessionStorage.getItem("tcsIdracLang");if(s)return s;}catch(e){}try{if(typeof tcsTop!=="undefined"&&tcsTop.lang)return tcsTop.lang;}catch(e){}return "en";})()).toLowerCase())"#,
    );
    let t = t.replace("window.opener.top", "tcsTop");
    // Avocent cert dialog is window.open of a 700x500 page; in TCS it shows
    // "undefined" and blocks KVM until dismissed without accepting the cert.
    let t = t.replace(
        "//  htmlViewer.disableRPCertPopup();",
        "htmlViewer.disableRPCertPopup();",
    );
    let t = t.replace(
        "htmlViewer.connectRPViewer();",
        "try{htmlViewer.disableRPCertPopup();}catch(e){} htmlViewer.connectRPViewer();",
    );
    // Avocent builds wss://ip:kvmPort/. Point host at TCS:443 and the path
    // at our RFB relay so the browser never dials port 5900.
    let t = t.replace(
        "htmlViewer.setRPServerConfiguration(mIPAddress, mPort);",
        r#"try{mIPAddress=location.hostname;}catch(e){}var _tcsKvmPort=mPort||"5900";var _tcsPath=(typeof tcsAbs==="function"?tcsAbs("__rfb/"+_tcsKvmPort):("__rfb/"+_tcsKvmPort));if(_tcsPath.charAt(0)==="/")_tcsPath=_tcsPath.substring(1);try{htmlViewer.setRPServerPath(_tcsPath);}catch(e){}try{htmlViewer.setRPServerConfiguration(mIPAddress, (location.port||(location.protocol==="https:"?"443":"80")));}catch(e){try{htmlViewer.setRPServerConfiguration(mIPAddress, _tcsKvmPort);}catch(e2){}}"#,
    );
    let t = t.replace(
        r#"parent.document.getElementById("navigationBar")"#,
        r#"tcsTop.document.getElementById("navigationBar")"#,
    );
    rewrite_top_to_idrac_root(&t)
}

/// Child-frame JS uses the identifier `top` for index.html state
/// (`top.snb`, `top.TOKEN_VALUE`, `eval("top."+…)`). `window.top` cannot
/// be redefined, so rewrite those reads onto `tcsTop`. Do not match `.top`
/// (CSS `style.top` / `vertical-align: top`). Do **not** rewrite `parent.`
/// globally — that raced the frameset load in 0.5.68.
fn rewrite_top_to_idrac_root(text: &str) -> String {
    let dot = regex::Regex::new(r"(^|[^.\w$])top\.").expect("idrac top-dot");
    let t = dot.replace_all(text, "${1}tcsTop.").into_owned();
    let bracket = regex::Regex::new(r"(^|[^.\w$])top\[").expect("idrac top-bracket");
    let t = bracket.replace_all(&t, "${1}tcsTop[").into_owned();
    // `tcsTop.lsnb.location.replace("lsnb.html")` → tcsNav("lsnb", …) so the
    // URL is forced onto the session prefix. Named-frame `window.lsnb` is
    // flaky in Chrome; tcsNav falls back to getElementsByName.
    let nav = regex::Regex::new(r"tcsTop\.([A-Za-z_][A-Za-z0-9_]*)\.location\.replace\(")
        .expect("idrac tcsNav replace");
    let t = nav.replace_all(&t, "tcsNav(\"$1\", ").into_owned();
    // Named frames on the frameset window are often the <frame> element, not
    // the content window — `tcsTop.lsnb.f_getHTML` is then undefined and menu
    // clicks no-op. Always go through tcsFrame() which uses contentWindow.
    let frames = regex::Regex::new(
        r"tcsTop\.(da|lsnb|snb|treelist|globalnav|title|logo|hidden_frame|licenseframe|unlicensedBanner|virtual|blank)\.",
    )
    .expect("idrac frame alias");
    let t = frames.replace_all(&t, "tcsFrame(\"$1\").").into_owned();
    let parent_frames = regex::Regex::new(r"(^|[^.\w$])parent\.(snb|da|lsnb|treelist)\.")
        .expect("idrac parent frame");
    let t = parent_frames
        .replace_all(&t, "${1}tcsFrame(\"$2\").")
        .into_owned();
    // Child frames often cannot read sibling contentWindow (sandbox / XHTML
    // frameset). Relay method calls through the frameset via postMessage.
    let call = regex::Regex::new(
        r#"tcsFrame\("(snb|lsnb|da|treelist|globalnav)"\)\.([A-Za-z_][A-Za-z0-9_]*)\("#,
    )
    .expect("idrac tcsCall");
    call.replace_all(&t, "tcsCall(\"$1\",\"$2\",")
        .into_owned()
}

fn rewrite_attr_slash_urls(text: &str, pfx: &str) -> String {
    let re = regex::Regex::new(
        r#"(?i)(\b(?:src|href|action|poster|data-src|data-href)\s*=\s*["'])(/[^"']*)"#,
    )
    .expect("attr rewrite regex");
    re.replace_all(text, |caps: &regex::Captures| {
        let val = &caps[2];
        if val.starts_with("//") || val.starts_with(pfx) {
            caps[0].to_string()
        } else {
            format!("{}{}{}", &caps[1], pfx, val)
        }
    })
    .into_owned()
}

pub fn apply_rewrites(path: &str, ctype: &str, body: &[u8], prefix: &str, bmc_host: &str) -> Vec<u8> {
    let name = path.rsplit('/').next().unwrap_or(path);
    // `/session` and `/data` are XML/JSON APIs. Rewriting them as HTML turns a
    // login-page fallback into a poisoned payload and the UI spinner never stops.
    if name == "session" || name == "data" || ctype.contains("xml") || ctype.contains("json") {
        return body.to_vec();
    }
    let is_text = ctype.contains("javascript")
        || ctype.contains("ecmascript")
        || ctype.contains("html")
        || ctype.contains("css")
        || ctype.contains("text/")
        || name.ends_with(".js")
        || name.ends_with(".html")
        || name.ends_with(".css")
        || name.ends_with(".jsesp")
        || path.is_empty()
        || path == "console"
        || path.starts_with("console?");
    if !is_text {
        return body.to_vec();
    }
    let mut text = String::from_utf8_lossy(body).into_owned();
    text = neutralize_idrac_framebust(&text);
    text = rewrite_absolute_urls(&text, prefix, bmc_host);
    // iDRAC 7 serves `functions.jsesp` as text/html. Injecting <base>/<script>
    // into it makes the file unparseable as JS and the da frame never leaves
    // blankLoading.html.
    let looks_like_script = name.ends_with(".js")
        || name.ends_with(".jsesp")
        || ctype.contains("javascript")
        || ctype.contains("ecmascript");
    let is_html = !looks_like_script
        && (ctype.contains("html")
            || name.ends_with(".html")
            || path.is_empty()
            || path == "console"
            || path.starts_with("console?")
            || text.trim_start().to_ascii_lowercase().starts_with("<!doctype")
            || text.trim_start().to_ascii_lowercase().starts_with("<html"));
    if looks_like_script {
        text = format!("{}{text}", websocket_hook_js(prefix));
        // Safari workers keep a built-in `WebSocket` binding; assigning
        // `self.WebSocket` does not change `new WebSocket(...)`.
        text = text.replace("new WebSocket(", "new (self.__tcsWS||WebSocket)(");
    }
    if is_html {
        text = inject_prefix_hooks(&text, prefix);
        text = inject_heartbeat(&text);
        // `<base href>` makes `href="#"` navigate to the prefix URL (or TCS).
        text = text.replace("href=\"#\"", "href=\"javascript:void(0)\"");
        text = text.replace("href='#'", "href='javascript:void(0)'");
        // Load the content pane immediately; child-frame JS often never
        // replaces blankLoading.html when sibling contentWindow is opaque.
        text = text.replace(
            r#"<frame src="blankLoading.html" name="da""#,
            r#"<frame src="sysSummaryData.html" name="da""#,
        );
        text = text.replace(
            r#"<frame src="sysSummary.html" name="da""#,
            r#"<frame src="sysSummaryData.html" name="da""#,
        );
        text = text.replace(
            r#"rows="37,23,*,0""#,
            r#"rows="37,23,80%,0""#,
        );
        text = text.replace(
            r#"id="contentArea" style="display:none;""#,
            r#"id="contentArea" style="display:block;""#,
        );
        // Nested sysSummary iframe is position:fixed vs the viewport, so in
        // a frameset it paints off-screen and the da pane looks white.
        text = text.replace(
            "iframe { position:fixed; display:block; width:100%; border:none; }",
            "iframe { position:absolute; top:0; left:0; display:block; width:100%; height:100%; border:none; }",
        );
    }
    text.into_bytes()
}

fn inject_heartbeat(text: &str) -> String {
    const HEARTBEAT: &str = r#"<script>(function(){function p(e){try{parent.postMessage({tcsIdrac:e||"alive"},"*")}catch(x){}}p("ready");setInterval(function(){p("alive")},2000);window.addEventListener("pagehide",function(){p("down")});})();</script>"#;
    if text.contains("tcsIdrac") {
        return text.to_string();
    }
    if let Some(idx) = text.rfind("</body>").or_else(|| text.rfind("</BODY>")) {
        let mut out = text.to_string();
        out.insert_str(idx, HEARTBEAT);
        return out;
    }
    format!("{text}{HEARTBEAT}")
}

pub async fn fetch_upstream(
    sess: &IdracSession,
    method: &Method,
    rel: &str,
    query: Option<&str>,
    extra: &HeaderMap,
    body: Option<Vec<u8>>,
) -> Result<(u16, String, Vec<u8>, Vec<(String, String)>, Vec<(String, String)>), AppError> {
    let origin = bmc_origin(&sess.bmc_host);
    let rel = rel.trim_start_matches('/');
    let mut url = if rel.is_empty() {
        format!("{}{}", origin, sess.viewer_path)
    } else {
        format!("{origin}/{rel}")
    };
    if let Some(q) = query {
        if !url.contains('?') {
            url.push('?');
            url.push_str(q);
        }
    }

    let client = gui_client(sess.use_native_tls)?;
    let mut req = client.request(
        reqwest::Method::from_bytes(method.as_str().as_bytes()).unwrap_or(reqwest::Method::GET),
        &url,
    );
    req = req.header("User-Agent", CHROME_UA);
    req = req.header(
        "Accept",
        extra
            .get(header::ACCEPT)
            .and_then(|h| h.to_str().ok())
            .unwrap_or("text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"),
    );
    req = req.header("Accept-Language", "en-US,en;q=0.9");
    if let Some(c) = sess.cookie_header() {
        req = req.header("Cookie", c);
    }
    if let Some(t) = &sess.x_auth_token {
        req = req.header("X-Auth-Token", t);
    }
    // iDRAC 7 `/session` returns the login HTML unless ST2 is sent as a
    // *header* (not a query param). The UI JS sets it from
    // `window.location.href`, which child frames don't have — so we always
    // attach the tokens from the login `forwardUrl`.
    req = req.header("Origin", &origin);
    req = req.header("Referer", format!("{}{}", origin, sess.viewer_path));
    for name in [
        "content-type",
        "xsrf-token",
        "x-requested-with",
        "x-csrf-token",
        "accept-language",
    ] {
        if let Some(v) = extra.get(name).and_then(|h| h.to_str().ok()) {
            if !v.is_empty() {
                req = req.header(name, v);
            }
        }
    }
    // Always win over a child-frame ST2 parsed from a URL that has no token.
    if let Some((st1, st2)) = super::idrac::parse_st_tokens(&sess.viewer_path) {
        req = req.header("ST1", st1);
        req = req.header("ST2", st2);
    }
    if let Some(b) = body {
        req = req.body(b);
    }

    let resp = req
        .send()
        .await
        .map_err(|e| AppError::Network(format!("iDRAC fetch {url}: {e}")))?;
    let status = resp.status().as_u16();
    let set_cookies = super::idrac::collect_cookies(resp.headers());
    let ctype = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|h| h.to_str().ok())
        .map(|s| s.split(';').next().unwrap_or("application/octet-stream").trim().to_string())
        .unwrap_or_else(|| "application/octet-stream".into());
    let pass_headers = pass_through_response_headers(resp.headers());
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| AppError::Network(format!("iDRAC body: {e}")))?
        .to_vec();
    Ok((status, ctype, bytes, set_cookies, pass_headers))
}

fn pass_through_response_headers(h: &reqwest::header::HeaderMap) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (name, val) in h.iter() {
        let n = name.as_str();
        if n.eq_ignore_ascii_case("x_language")
            || n.eq_ignore_ascii_case("x-language")
            || n.eq_ignore_ascii_case("content-disposition")
        {
            if let Ok(v) = val.to_str() {
                out.push((n.to_string(), v.to_string()));
            }
        }
    }
    out
}

pub fn into_response(
    status: u16,
    ctype: String,
    body: Vec<u8>,
    set_cookies: Vec<String>,
    pass_headers: Vec<(String, String)>,
) -> Result<Response, AppError> {
    let mut builder = Response::builder()
        .status(StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR));
    builder = builder.header(header::CONTENT_TYPE, ctype);
    builder = builder.header("X-Content-Type-Options", "nosniff");
    builder = builder.header("X-Frame-Options", "SAMEORIGIN");
    builder = builder.header("Referrer-Policy", "no-referrer");
    let mut has_lang = false;
    for (n, v) in &pass_headers {
        if n.eq_ignore_ascii_case("x_language") || n.eq_ignore_ascii_case("x-language") {
            has_lang = true;
        }
        builder = builder.header(n.as_str(), v);
    }
    // iDRAC 7 index.html does getResponseHeader('X_Language').substring(0,2)
    // and throws if the header is missing — the UI then never leaves the
    // blankLoading spinner. Always provide a language header.
    if !has_lang {
        builder = builder.header("X_Language", "en-US");
    }
    let mut res = builder
        .body(Body::from(body))
        .map_err(|e| AppError::Internal(format!("iDRAC asset response: {e}")))?;
    for c in set_cookies {
        if let Ok(v) = HeaderValue::from_str(&c) {
            res.headers_mut().append(header::SET_COOKIE, v);
        }
    }
    Ok(res)
}

/// Rewrite an upstream Set-Cookie so it is scoped to the TCS session prefix.
pub fn rewrite_set_cookie(raw_name: &str, raw_value: &str, prefix: &str) -> String {
    let pfx = prefix.trim_end_matches('/');
    format!("{raw_name}={raw_value}; Path={pfx}/; SameSite=Lax")
}

pub fn parse_rel_from_uri(full_path: &str, sid: &str) -> String {
    let rel = match full_path.rsplit_once(&format!("/console/{sid}")) {
        Some((_, rest)) => rest.trim_start_matches('/').to_string(),
        None => String::new(),
    };
    if rel.starts_with("__rfb/") {
        rel.trim_end_matches('/').to_string()
    } else {
        rel
    }
}

pub fn is_websocket_request(headers: &HeaderMap) -> bool {
    headers
        .get(header::UPGRADE)
        .and_then(|h| h.to_str().ok())
        .map(|s| s.eq_ignore_ascii_case("websocket"))
        .unwrap_or(false)
}

pub async fn relay_ws(session: IdracSession, socket: WebSocket, rel: String, query: Option<String>, proto: Option<String>) {
    tracing::info!(rel = %rel, proto = ?proto, "iDRAC websocket relay start");
    let origin = bmc_origin(&session.bmc_host);
    let ws_origin = origin
        .replace("https://", "wss://")
        .replace("http://", "ws://");

    let upstream = if let Some(port) = rel.strip_prefix("__rfb/") {
        // Prefer TLS on the remote-presence port, then plain WS.
        let tls = format!("wss://{}:{port}/", host_only(&session.bmc_host));
        let plain = format!("ws://{}:{port}/", host_only(&session.bmc_host));
        match connect_ws(&tls, &session, &origin, proto.as_deref()).await {
            Ok(s) => s,
            Err(e1) => match connect_ws(&plain, &session, &origin, proto.as_deref()).await {
                Ok(s) => s,
                Err(e2) => {
                    tracing::warn!(error1 = %e1, error2 = %e2, "iDRAC RFB upstream failed");
                    let _ = socket;
                    return;
                }
            },
        }
    } else {
        let mut url = format!("{}/{}", ws_origin.trim_end_matches('/'), rel.trim_start_matches('/'));
        if let Some(q) = &query {
            if !url.contains('?') {
                url.push('?');
                url.push_str(q);
            }
        }
        match connect_ws(&url, &session, &origin, proto.as_deref()).await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, url = %url, "iDRAC KVM upstream connect failed");
                let _ = socket;
                return;
            }
        }
    };

    bridge(socket, upstream).await;
}

fn host_only(host: &str) -> String {
    let h = host.split('/').next().unwrap_or(host);
    if let Some(rest) = h.strip_prefix('[') {
        if let Some(end) = rest.find(']') {
            return rest[..end].to_string();
        }
    }
    if let Some((name, port)) = h.rsplit_once(':') {
        if !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()) {
            return name.to_string();
        }
    }
    h.to_string()
}

async fn connect_ws(
    url: &str,
    session: &IdracSession,
    origin: &str,
    subprotocol: Option<&str>,
) -> Result<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>, String> {
    let mut req = url.into_client_request().map_err(|e| e.to_string())?;
    {
        let h = req.headers_mut();
        if let Some(c) = session.cookie_header() {
            if let Ok(v) = c.parse() {
                h.insert("Cookie", v);
            }
        }
        if let Some(t) = &session.x_auth_token {
            if let Ok(v) = t.parse() {
                h.insert("X-Auth-Token", v);
            }
        }
        if let Ok(v) = origin.parse() {
            h.insert("Origin", v);
        }
        h.insert("User-Agent", CHROME_UA.parse().unwrap());
        if let Some(p) = subprotocol {
            if let Ok(v) = p.parse() {
                h.insert("Sec-WebSocket-Protocol", v);
            }
        }
    }
    let connector = insecure_connector();
    match connect_async_tls_with_config(req, None, false, connector).await {
        Ok((stream, _resp)) => Ok(stream),
        Err(e) => Err(format!("iDRAC WS {url}: {e}")),
    }
}

async fn bridge(
    socket: WebSocket,
    up: tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
) {
    let (mut browser_tx, mut browser_rx) = socket.split();
    let (mut up_tx, mut up_rx) = up.split();

    let to_up = tokio::spawn(async move {
        while let Some(Ok(msg)) = browser_rx.next().await {
            let out = match msg {
                Message::Binary(b) => TwsMsg::Binary(b.to_vec().into()),
                Message::Text(t) => TwsMsg::Text(t.as_str().to_string().into()),
                Message::Ping(p) => TwsMsg::Ping(p.to_vec().into()),
                Message::Pong(p) => TwsMsg::Pong(p.to_vec().into()),
                Message::Close(_) => break,
            };
            if up_tx.send(out).await.is_err() {
                break;
            }
        }
    });

    let to_browser = tokio::spawn(async move {
        while let Some(Ok(msg)) = up_rx.next().await {
            let out = match msg {
                TwsMsg::Binary(b) => Message::Binary(b.to_vec().into()),
                TwsMsg::Text(t) => Message::Text(t.to_string().into()),
                TwsMsg::Ping(p) => Message::Ping(p.to_vec().into()),
                TwsMsg::Pong(p) => Message::Pong(p.to_vec().into()),
                TwsMsg::Close(_) => break,
                TwsMsg::Frame(_) => continue,
            };
            if browser_tx.send(out).await.is_err() {
                break;
            }
        }
    });

    let _ = to_browser.await;
    to_up.abort();
}

pub fn max_body() -> usize {
    MAX_BODY
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_safety() {
        assert!(is_safe_idrac_path(""));
        assert!(is_safe_idrac_path("console"));
        assert!(is_safe_idrac_path("restgui/vue/index.html"));
        assert!(is_safe_idrac_path("__rfb/5900"));
        assert!(is_safe_idrac_path("__rfb/5900/"));
        assert!(!is_safe_idrac_path("../etc/passwd"));
        assert!(!is_safe_idrac_path("foo/../../bar"));
        assert!(!is_safe_idrac_path("__rfb/nope"));
    }

    #[test]
    fn rewrite_attrs_and_abs() {
        let html = r#"<script src="/restgui/app.js"></script><a href="/console">x</a>"#;
        let out = rewrite_absolute_urls(html, "/api/machines/m/console/idrac_abc", "10.0.0.5");
        assert!(out.contains("/api/machines/m/console/idrac_abc/restgui/app.js"));
        assert!(out.contains("/api/machines/m/console/idrac_abc/console"));
    }

    #[test]
    fn syssummary_hides_progress_for_stock_gui() {
        let js = "function hideProgress() {\n   \t\tif (loading)\n   \t\t{\n\t\t\tif(top.isCustomGui == 1){\n\t\t\t\tprogressBar.hide();\n\t\t\t}\n   \t\t}\n   }";
        let out = neutralize_idrac_framebust(js);
        assert!(out.contains("progressBar.hide();"), "{out}");
        assert!(out.contains("progressPage"), "{out}");
        assert!(!out.contains("if(top.isCustomGui == 1){\n\t\t\t\tprogressBar.hide();"));
    }

    #[test]
    fn syssummary_iframe_not_fixed() {
        let html = "<html><head></head><body><style>iframe { position:fixed; display:block; width:100%; border:none; }</style></body></html>";
        let out = apply_rewrites(
            "sysSummary.html",
            "text/html",
            html.as_bytes(),
            "/api/machines/m/console/idrac_abc",
            "10.0.0.5",
        );
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("position:absolute"), "{s}");
        assert!(!s.contains("position:fixed"));
    }

    #[test]
    fn da_frame_starts_on_syssummary() {
        let html = r#"<html><head></head><frameset><frame src="blankLoading.html" name="da" scrolling="auto" /></frameset></html>"#;
        let out = apply_rewrites(
            "index.html",
            "text/html",
            html.as_bytes(),
            "/api/machines/m/console/idrac_abc",
            "10.0.0.5",
        );
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains(r#"<frame src="sysSummaryData.html" name="da""#), "{s}");
        assert!(!s.contains(r#"src="blankLoading.html" name="da""#));
    }

    #[test]
    fn inject_has_prefix() {
        let html = "<html><head></head><body></body></html>";
        let out = inject_prefix_hooks(html, "/api/machines/m/console/idrac_abc");
        assert!(out.contains(r#"var P="/api/machines/m/console/idrac_abc""#));
        assert!(out.contains(r#"<base href="/api/machines/m/console/idrac_abc/">"#));
        assert!(out.contains("tcsHideProgress"));
        assert!(out.contains("#contentArea"));
        assert!(out.contains("virtualconsolehtml5.html"), "{out}");
        assert!(out.contains("__tcsKvmLaunched"), "{out}");
    }

    #[test]
    fn rel_from_uri() {
        let p = "/api/machines/111/console/idrac_abc/restgui/foo.js";
        assert_eq!(parse_rel_from_uri(p, "idrac_abc"), "restgui/foo.js");
        let p = "/api/machines/111/console/idrac_abc";
        assert_eq!(parse_rel_from_uri(p, "idrac_abc"), "");
        let p = "/api/machines/111/console/idrac_abc/__rfb/5900/";
        assert_eq!(parse_rel_from_uri(p, "idrac_abc"), "__rfb/5900");
    }

    #[test]
    fn checktop_neutralized() {
        let js = "function CheckTop()\n{\n\t\tif ( top.document.location.href.search('index') < 0 )\n\t\t{\n\t\t\t\ttop.document.location.href = \"/start.html\";\n";
        let out = neutralize_idrac_framebust(js);
        assert!(out.contains("if ( false )"));
        assert!(!out.contains("if ( top.document.location.href.search('index') < 0 )"));
        assert!(!out.contains("top.document.location.href ="));
        assert!(out.contains("tcsNoop = \"/start.html\""));
    }

    #[test]
    fn top_location_replace_neutralized() {
        let js = r#"if (top.frames.length >= 1) top.location.replace("/sclogin.html"); parent.location = "/login.html";"#;
        let out = neutralize_idrac_framebust(js);
        assert!(out.contains("if (false)"));
        assert!(!out.contains("top.location.replace("));
        assert!(out.contains(r#"void ("/sclogin.html")"#));
        assert!(out.contains(r#"tcsNoop = "/login.html""#));
    }

    #[test]
    fn top_identifier_rewritten_to_tcs_top() {
        let js = r#"lookup = top.treelist.Lookup; eval("top." + name); top.snb.f_getHTML(cat); el.style.top = "0"; vertical-align: top;"#;
        let out = neutralize_idrac_framebust(js);
        assert!(out.contains(r#"tcsFrame("treelist").Lookup"#), "{out}");
        assert!(out.contains(r#"eval("tcsTop." + name)"#));
        assert!(out.contains(r#"tcsCall("snb","f_getHTML","#), "{out}");
        assert!(out.contains(r#"el.style.top = "0""#));
        assert!(out.contains("vertical-align: top;"));
        assert!(!out.contains("top.treelist"));
        assert!(!out.contains("tcsTop.treelist"));
    }

    #[test]
    fn lsnb_and_da_navigation_prefixed() {
        let js = r#"top.lsnb.location.replace("lsnb.html"); parent.da.location = link; parent.snb.f_getHTML(cat);"#;
        let out = neutralize_idrac_framebust(js);
        assert!(out.contains(r#"tcsNav("lsnb", "lsnb.html")"#), "{out}");
        assert!(out.contains(r#"tcsNav("da", link)"#), "{out}");
        let js2 = r#"parent.snb.f_getHTML(newCat, tab, id, flag);"#;
        let out2 = neutralize_idrac_framebust(js2);
        assert!(out2.contains("tcsOpenCat(newCat,tab,id)"), "{out2}");
        let launch = "function launchKVM(arg) {\n\t\t\tlaunchConsole(isKVMEnabled, arg);\n\t\t}";
        let out3 = neutralize_idrac_framebust(launch);
        assert!(out3.contains("virtualconsolehtml5.html"), "{out3}");
        assert!(out3.contains("tcsNav"), "{out3}");
        assert!(out3.contains("TOKEN_VALUE"), "{out3}");
        assert!(out3.contains("aimSession"), "{out3}");
        assert!(!out3.contains("launchConsole(isKVMEnabled"));
        assert!(out.contains(r#"tcsCall("snb","f_getHTML","#), "{out}");
        assert!(!out.contains("parent.da.location"));
        assert!(!out.contains("parent.snb.f_getHTML"));
    }

    #[test]
    fn html5_viewer_uses_frameset_locale_and_skips_cert_popup() {
        let js = concat!(
            "localeObj1 = window.opener.top.localeObj;\n",
            "localeObj = window.opener.top.localeObj;\n",
            "var strLanguage = window.opener.top.lang.toLowerCase();\n",
            "       //  htmlViewer.disableRPCertPopup();\n",
            "        htmlViewer.connectRPViewer();\n",
            "\t\t\t\t\t\ttop.htmlViewerWindow = window.open(htmlURL,'','toolbar=no,menubar=no,status=no,location=yes,resizable ');\n",
        );
        let out = neutralize_idrac_framebust(js);
        assert!(out.contains("viewer_connectingViewer"), "{out}");
        assert!(out.contains("btn_ok"), "{out}");
        assert!(!out.contains("window.opener.top"), "{out}");
        assert!(out.contains("disableRPCertPopup()"), "{out}");
        assert!(!out.contains("//  htmlViewer.disableRPCertPopup"), "{out}");
        assert!(out.contains(r#"tcsNav("da", htmlURL)"#), "{out}");
        assert!(!out.contains("window.open(htmlURL"), "{out}");
        let cfg = neutralize_idrac_framebust(
            "htmlViewer.setRPServerConfiguration(mIPAddress, mPort);\nhtmlViewer.connectRPViewer();",
        );
        assert!(cfg.contains("setRPServerPath"), "{cfg}");
        assert!(cfg.contains("__rfb/"), "{cfg}");
    }

    #[test]
    fn jsesp_not_html_injected() {
        let js = b"if(typeof(top.ssnObj) != \"undefined\") { var x = top.localeObj['btn_ok']; }";
        let out = apply_rewrites(
            "functions.jsesp",
            "text/html",
            js,
            "/api/machines/m/console/idrac_abc",
            "10.0.0.5",
        );
        let s = String::from_utf8(out).unwrap();
        assert!(!s.contains("<base "));
        assert!(!s.contains("<script>"));
        assert!(s.contains("tcsTop.ssnObj"));
        assert!(s.contains("tcsTop.localeObj"));
    }

    #[test]
    fn worker_js_gets_websocket_rfb_hook() {
        let out = apply_rewrites(
            "js/ViewerJS/viewer/websockethandler.js",
            "application/javascript",
            b"function createSocket(){return new WebSocket('wss://bmc:5900/');}",
            "/api/machines/m/console/idrac_abc",
            "10.0.0.5",
        );
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("__tcsRfb"), "{s}");
        assert!(s.contains("/__rfb/"), "{s}");
        assert!(s.contains("createSocket"), "{s}");
        assert!(s.contains("self.__tcsWS||WebSocket"), "{s}");
        assert!(!s.contains("new WebSocket("), "{s}");
    }

    #[test]
    fn session_api_not_rewritten() {
        let json = br#"{ "getSsnVar" : { "loginToDRAC" :"1" } }"#;
        let out = apply_rewrites(
            "session",
            "application/json",
            json,
            "/api/machines/m/console/idrac_abc",
            "10.0.0.5",
        );
        assert_eq!(out, json);
    }
}
