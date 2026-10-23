if (typeof queueMicrotask === "undefined") {
window.queueMicrotask = function(callback) {
Promise.resolve().then(callback).catch(e => setTimeout(() => { throw e; }));
};
}
;
/* <![CDATA[ */
"use strict";function _typeof(t){return(_typeof="function"==typeof Symbol&&"symbol"==typeof Symbol.iterator?function(t){return typeof t}:function(t){return t&&"function"==typeof Symbol&&t.constructor===Symbol&&t!==Symbol.prototype?"symbol":typeof t})(t)}!function(){var t=function(){var t,e,o=[],n=window,r=n;for(;r;){try{if(r.frames.__tcfapiLocator){t=r;break}}catch(t){}if(r===n.top)break;r=r.parent}t||(!function t(){var e=n.document,o=!!n.frames.__tcfapiLocator;if(!o)if(e.body){var r=e.createElement("iframe");r.style.cssText="display:none",r.name="__tcfapiLocator",e.body.appendChild(r)}else setTimeout(t,5);return!o}(),n.__tcfapi=function(){for(var t=arguments.length,n=new Array(t),r=0;r<t;r++)n[r]=arguments[r];if(!n.length)return o;"setGdprApplies"===n[0]?n.length>3&&2===parseInt(n[1],10)&&"boolean"==typeof n[3]&&(e=n[3],"function"==typeof n[2]&&n[2]("set",!0)):"ping"===n[0]?"function"==typeof n[2]&&n[2]({gdprApplies:e,cmpLoaded:!1,cmpStatus:"stub"}):o.push(n)},n.addEventListener("message",(function(t){var e="string"==typeof t.data,o={};if(e)try{o=JSON.parse(t.data)}catch(t){}else o=t.data;var n="object"===_typeof(o)&&null!==o?o.__tcfapiCall:null;n&&window.__tcfapi(n.command,n.version,(function(o,r){var a={__tcfapiReturn:{returnValue:o,success:r,callId:n.callId}};t&&t.source&&t.source.postMessage&&t.source.postMessage(e?JSON.stringify(a):a,"*")}),n.parameter)}),!1))};"undefined"!=typeof module?module.exports=t:t()}();
;
window._sp_queue = [];
window._sp_ = {
config: {
accountId: 375,
baseEndpoint: 'https://cdn.privacy-mgmt.com',
propertyId: 37937,
gdpr: { },
events: {
onMessageReady: function() {
// console.log('[event] onMessageReady', arguments);
if (typeof globalThis.IOMm === 'function') {
globalThis.IOMm('pageview', { cp: 'Modal' });
} else {
console.warn('IOMm ist nicht verfügbar – Pageview wurde nicht erfasst.');
}
}
/*     ,
onMessageReceiveData: function() {
console.log('[event] onMessageReceiveData', arguments);
},
onSPReady: function() {
console.log('[event] onSPReady', arguments);
},
onError: function() {
console.log('[event] onError', arguments);
},
onMessageChoiceSelect: function() {
console.log('[event] onMessageChoiceSelect', arguments);
},
onConsentReady: function(consentUUID, euconsent) {
console.log('[event] onConsentReady', arguments);
},
onPrivacyManagerAction: function() {
console.log('[event] onPrivacyManagerAction', arguments);
},
onPMCancel: function() {
console.log('[event] onPMCancel', arguments);
} */
}
}
}
/* ]]> */
;
// scoping
if (!window.marOtaz_de) window.marOtaz_de = new Object;
marOtaz_de.isMobile = ()=> {
if (/Mobi|Android/i.test(navigator.userAgent) && screen.width < 768) {
return true
} else {
return false;
}
}
// to handle ros top right
marOtaz_de.isBigDesktopScreen = ()=> {
if ( screen.width >= 1600) {
return true
} else {
return false;
}
}
// early init of alpine store for simpler access to page view states
document.addEventListener('alpine:init', () => {
Alpine.store('campaign', {
pagesShow: {'initial-display': true},
bountyState: () => {
return ''}
})
});
var st = marOtaz_de.isMobile() ? 'mobtaz' : 'taz';
var dn = marOtaz_de.isMobile() ? 'data-502d5cce76.taz.de' : 'data-2d3a3249cd.taz.de'
if (typeof globalThis.IOMm === 'function') {
globalThis.IOMm('configure', {
st,
dn,
mh: 5
});
} else {
console.warn('IOMm ist nicht verfügbar – Konfiguration wurde nicht ausgeführt.');
}
;
// IVW pageview ;)
if (typeof globalThis.IOMm === 'function') {
globalThis.IOMm('pageview', {
cp: 'Startseite'
});
} else {
console.warn('IOMm ist nicht verfügbar – Pageview wurde nicht erfasst.');
}
;
if (typeof globalThis.IOMm === 'function') {
globalThis.IOMm('3p', 'qds', st);
} else {
console.warn('IOMm ist nicht verfügbar – 3p-Aufruf wurde nicht ausgeführt.');
}
// console.log(st);
// console.log(dn);
// polyfill alpine
if (typeof window.queueMicrotask !== "function") {
window.queueMicrotask = function (callback) {
Promise.resolve()
.then(callback)
.catch(e => setTimeout(() => { throw e; }));
};
}
function loadScript(url) {
const script = document.createElement('script');
script.src = url;
script.type = 'text/javascript';
document.head.appendChild(script);
}
/********* END LAZY LOAD ********/
/* document.addEventListener("DOMContentLoaded", function() {
marOtaz_de.lazyObserver.observe();
});
*/
;
var googletag = googletag || {}; googletag.cmd = googletag.cmd || [];
(function(){
if( false == 'true' ) return;
const adArea =  'Startseite';
const subposition = '';
var page_id = '';
const tags = [];
// tag tageting:
//console.log('page_id:',[page_id]);
//console.log('tag for tag-sites or tags for article-site:', tags);
//console.log('at which adArea:', adArea);
googletag.cmd.push( function() {
try {
var pa = googletag.pubads();
if (typeof pa !== 'object') throw new Error('pubads not available');
pa.setForceSafeFrame(true);
// https://developers.google.com/publisher-tag/guides/minimize-layout-shift?hl=de
pa.collapseEmptyDivs();
pa.setTargeting('position', [ adArea ]);
pa.setTargeting('subposition', [ subposition ]);
//pa.setTargeting('test', ['fireplace']);
//pa.setTargeting('test', ['wallpaper']);
pa.setTargeting('page_id', [page_id]);
if (tags !== undefined) pa.setTargeting('taz_tag',  tags);
if( location.protocol =='https' )
pa.setTargeting('ssl', ['ja']);
googletag.enableServices();
} catch (e) {
// Error indicates the script was blocked
console.log('Error in gpt activation', e);
if (document.body) {
document.body.classList.add('prevent-ad-empty-space');
} else {
document.addEventListener('DOMContentLoaded', () => {
document.body.classList.add('prevent-ad-empty-space');
});
}
}
});
})();
(function () {
if (!window.filOtaz_de) window.filOtaz_de = Object;
if (!window.filOtaz_de.ads) window.filOtaz_de.ads = Object;
if (!window.filOtaz_de.ads.gpt) window.filOtaz_de.ads.gpt = Object;
var sizes = JSON.parse('{"taz.de/taz.de_artikel_1":[[300,250],[728,90],[624,150],[970,90],[770,250],[800,250],[970,250]],"taz.de/taz.de_artikel_2":[[300,250],[728,90],[624,150],[970,90],[770,250],[800,250],[970,250]],"taz.de/taz.de_artikel_3":[[300,250],[728,90],[624,150],[970,90],[770,250],[800,250],[970,250]],"taz.de/taz.de_artikel_4":[[300,250],[728,90],[624,150],[970,90],[770,250],[800,250],[970,250]],"taz.de/taz.de_artikel_5":[[300,250],[728,90],[624,150],[970,90],[770,250],[800,250],[970,250]],"taz.de/taz.de_artikel_6":[[300,250],[728,90],[624,150],[970,90],[770,250],[800,250],[970,250]],"taz.de/taz.de_artikel_7":[[300,250],[728,90],[624,150],[970,90],[770,250],[800,250],[970,250]],"taz.de/taz.de_artikel_8":[[300,250],[728,90],[624,150],[970,90],[770,250],[800,250],[970,250]],"taz.de/taz.de_artikel_9":[[300,250],[728,90],[624,150],[970,90],[770,250],[800,250],[970,250]],"taz.de/taz.de_artikel_10":[[300,250],[728,90],[624,150],[970,90],[770,250],[800,250],[970,250]],"taz.de/taz.de_artikel_1-mobile":[[300,600],[336,280],[300,250],[320,100],[320,50]],"taz.de/taz.de_artikel_2-mobile":[[300,600],[336,280],[300,250],[320,100],[320,50]],"taz.de/taz.de_artikel_3-mobile":[[300,600],[336,280],[300,250],[320,100],[320,50]],"taz.de/taz.de_artikel_4-mobile":[[300,600],[336,280],[300,250],[320,100],[320,50]],"taz.de/taz.de_artikel_5-mobile":[[300,600],[336,280],[300,250],[320,100],[320,50]],"taz.de/taz.de_artikel_6-mobile":[[300,600],[336,280],[300,250],[320,100],[320,50]],"taz.de/taz.de_artikel_7-mobile":[[300,600],[336,280],[300,250],[320,100],[320,50]],"taz.de/taz.de_artikel_8-mobile":[[300,600],[336,280],[300,250],[320,100],[320,50]],"taz.de/taz.de_artikel_9-mobile":[[300,600],[336,280],[300,250],[320,100],[320,50]],"taz.de/taz.de_artikel_10-mobile":[[300,600],[336,280],[300,250],[320,100],[320,50]],"taz.de/taz.de_uebersicht-1":[[300,250],[728,90],[624,150],[970,90],[770,250],[800,250],[970,250]],"taz.de/taz.de_uebersicht-2":[[300,250],[728,90],[624,150],[970,90],[770,250],[800,250],[970,250]],"taz.de/taz.de_uebersicht-3":[[300,250],[728,90],[624,150],[970,90],[770,250],[800,250],[970,250]],"taz.de/taz.de_uebersicht-4":[[300,250],[728,90],[624,150],[970,90],[770,250],[800,250],[970,250]],"taz.de/taz.de_uebersicht-5":[[300,250],[728,90],[624,150],[970,90],[770,250],[800,250],[970,250]],"taz.de/taz.de_uebersicht-6":[[300,250],[728,90],[624,150],[970,90],[770,250],[800,250],[970,250]],"taz.de/taz.de_uebersicht-7":[[300,250],[728,90],[624,150],[970,90],[770,250],[800,250],[970,250]],"taz.de/taz.de_uebersicht-8":[[300,250],[728,90],[624,150],[970,90],[770,250],[800,250],[970,250]],"taz.de/taz.de_uebersicht-9":[[300,250],[728,90],[624,150],[970,90],[770,250],[800,250],[970,250]],"taz.de/taz.de_uebersicht-10":[[300,250],[728,90],[624,150],[970,90],[770,250],[800,250],[970,250]],"taz.de/taz.de_uebersicht-1-mobile":[[300,600],[336,280],[300,250],[320,100],[320,50]],"taz.de/taz.de_uebersicht-2-mobile":[[300,600],[336,280],[300,250],[320,100],[320,50]],"taz.de/taz.de_uebersicht-3-mobile":[[300,600],[336,280],[300,250],[320,100],[320,50]],"taz.de/taz.de_uebersicht-4-mobile":[[300,600],[336,280],[300,250],[320,100],[320,50]],"taz.de/taz.de_uebersicht-5-mobile":[[300,600],[336,280],[300,250],[320,100],[320,50]],"taz.de/taz.de_uebersicht-6-mobile":[[300,600],[336,280],[300,250],[320,100],[320,50]],"taz.de/taz.de_uebersicht-7-mobile":[[300,600],[336,280],[300,250],[320,100],[320,50]],"taz.de/taz.de_uebersicht-8-mobile":[[300,600],[336,280],[300,250],[320,100],[320,50]],"taz.de/taz.de_uebersicht-9-mobile":[[300,600],[336,280],[300,250],[320,100],[320,50]],"taz.de/taz.de_uebersicht-10-mobile":[[300,600],[336,280],[300,250],[320,100],[320,50]],"taz.de/taz.de_ros_top-links":[[300,600],[160,600],[200,600]],"taz.de/taz.de_ros_top-rechts":[[300,600],[160,600],[200,600]],"taz.de/taz.de_ros_top-mitte":[[300,250],[728,90],[624,150],[970,90],[770,250],[800,250],[970,250]],"taz.de/taz.de_ros_top-mitte-mobile":[[336,280],[300,250],[320,100],[320,50]],"taz.de/taz.de_fireplace-mitte":[[970,250],[970,90],[728,90]],"taz.de/taz.de_fireplace-links":[[300,600]],"taz.de/taz.de_fireplace-rechts":[[300,600]],"taz.de/taz.de_fireplace_bg":[1,1]}');
var roadblocks = {};
roadblocks = {
'ros_top-mitte': ['ros_billboard_1', 'fireplace-mitte']
, 'ros_top-mitte': ['fireplace-mitte']
, 'ros_top-rechts': ['fireplace-rechts']
, 'ros_top-links': ['fireplace-links']
};
filOtaz_de.ads.fitIn = function (container) {
var container = $(container);
if (!container.is('.ad_zone')) return;
var packing = container.parent('.ad_bin');
packing.addClass('shown');
container.addClass('ad_zone_shown');
if (container.hasClass('ad_zone_contentad')) {
packing.prev('.sect_leads').addClass('aded_' + (container.height() > 120 ? 'big' : 'small'));
var badge = container.hasClass('ad_zone_sold') ? 'Anzeige' : 'taz-Angebot';
if (container.prev('.ad_badge').length === 0) {
container.before('<div class="ad_badge mgb-xsmall typo-meta-data has-text-text-icon-grey is-flex is-justify-content-flex-end"><small>' + badge + ' </small></div>');
}
}
container.trigger('TAZadInserted').trigger('TAZboxChange');
};
filOtaz_de.ads.reclaim = function (container) {
// remove whitespace from non-delivered ad
var container = $(container);
//console.log('reclaim '+ container.attr('id') );
if (!container.is('.ad_zone')) return;
var packing = container.parent('.ad_bin');
packing.removeClass('shown');
container.removeClass('ad_zone_shown');
if (container.hasClass('ad_zone_contentad')) {
packing.prev('.sect_leads').removeClass('aded_big aded_small');
}
container.trigger('TAZadRemoved').trigger('TAZboxChange');
};
var fuzeAdFrame = function (frame) {
// register frame for messages, fire if one is from our content
console.log('legacy fuzeAdFrame', frame);
};
// Hintergrund per Message
var eventMethod = window.addEventListener ? "addEventListener" : "attachEvent";
var messageEvent = eventMethod == "attachEvent" ? "onmessage" : "message";
window[eventMethod](messageEvent, function (e) {
if (e.data.toString().indexOf("color") != -1) {
$("#background").css("background-color", e.data.split(" ")[1]);
}
}, false);
// activate ad frame
var adZoneHandled = {};
//console.log('activateAdFrame? handled adZone is:', adZoneHandled);
var get_id = function (name) {
return $('div[name=' + name + ']').attr('id');
};
var get_name = function (id) {
return $('#' + id).attr('name');
};
unblock = function (id, rb_id) {
var name = get_name(id);
var index = roadblocks[name].indexOf(rb_id);
if (index > -1) {
roadblocks[name].splice(index, 1);
}
if (!roadblocks[name].length) {
googletag.defineSlot('/53015287,118803185/' + name, sizes[name], id).addService(googletag.pubads());
googletag.display(id);
}
};
filOtaz_de.ads.gpt.activateAdFrame = function (el, id) {
if (!id || typeof el === 'undefined') return;
if (adZoneHandled[id]) return; adZoneHandled[id] = true;
// only one active zone per id
//console.log('activateAdFrame? handled adZone is:', adZoneHandled);
var packing = $(el);
var container = $(el).find('.ad_zone');
var name = id;
//console.log('name', name);
if (!name || !sizes[name]) return;
//console.log('1');
//console.log(sizes[name]);
// handle custom ad load events
container.on('filOtaz_de.ads.gpt.noBanner', function () { return false; });
container.on('filOtaz_de.ads.gpt.preBanner', function () {
filOtaz_de.ads.fitIn(container);
return false;
});
container.on('filOtaz_de.ads.gpt.postBanner', function () { return false; });
try {
googletag.cmd.push(function () {
var pa = googletag.pubads();
if (roadblocks[name]) {
pa.addEventListener('slotRenderEnded', function (e) {
for (var i = 0; i < roadblocks[name].length; i++) {
if (e.slot.getSlotElementId() ==  roadblocks[name][i]) {
if (e.isEmpty) {
unblock(id, roadblocks[name][i]);
}
else {
document.getElementById(id).style.display = 'none';
for (var key in roadblocks) {
if (!roadblocks.hasOwnProperty(key)) continue;
for (var j = 0; j < roadblocks[key].length; j++)
if (id ==  roadblocks[key][j]) {
unblock(get_id(key), roadblocks[key][j]);
break;
}
}
}
}
}
});
// pa.addEventListener
//console.log('roadblocks' + id);
googletag.display(id);
}
else {
pa.display('/53015287,118803185/' + name, sizes[name], id);
//console.log( 'asked for', '/53015287,118803185/'+ name, sizes[name], id );
}
// https://developers.google.com/publisher-tag/reference?hl=de
pa.addEventListener('slotRenderEnded', function (e) {
if (e.slot.getSlotElementId() != id) return;
// not our slot
container.trigger(e.isEmpty ?
'filOtaz_de.ads.gpt.noBanner' : 'filOtaz_de.ads.gpt.preBanner');
if (e.isEmpty) {
console.log("No ad delivered for slot:", e.slot.getSlotElementId());
container.addClass('isSlotRenderEndedEmpty');
container.closest('.ad_zone-container-inner').addClass('isSlotRenderEndedEmpty');
}
// added by mar prevent empty containers
container.addClass('isSlotRenderEnded');
container.closest('.ad_zone-container-inner').addClass('isSlotRenderEnded');
});
pa.addEventListener('slotResponseReceived', function (e) {
if (e.slot.getSlotElementId() != id) return;
container.addClass('slotResponseReceived')
});
pa.addEventListener('slotOnload', function (e) {
if (e.slot.getSlotElementId() != id) return;
container.addClass('slotOnload')
});
pa.addEventListener('slotRequested', function (e) {
if (e.slot.getSlotElementId() != id) return;
container.addClass('slotRequested')
});
});
} catch (err) {
$('body').addClass('prevent-ad-empty-space');
console.log('error in gpt activation', err)
}
}
console.time("AdLoad")
console.timeEnd("AdLoad")
})();
;
marOtaz_de.tziLayerOldFrontend = async (setting) => {
if ($('.tziFgContainer2021').length) return;
const isCMPtoDecideBeforeShowLayer = false; // todo: remove
const isDirPage = document.querySelector('body.isdir') !== null;
const isArticlePage = document.querySelector('body.isArticle') !== null;
//const pagetype = document.body.classList.contains('homepage') ? 'home' : 'article';
const isClosedByScrolling = false;
const layerListUuid = marOtaz_de.isMobile() ? '8b0790d4-53b5-43ba-9e14-e827d7f07de7' : 'd464a2b5-0c03-4ced-b9a6-51ad423f4e60';
const path = 'https://taz.de/!v=' + layerListUuid + '/c.xml';
const heightOfMinimizedLayer = 70;
const fetchAndParse = async (source) => {
try {
let response = await fetch(source);
let text = await response.text();
let parser = new DOMParser();
let parsed = parser.parseFromString(text, "application/xml");
return parsed;
} catch (error) {
console.log('tzi layer fetch', error);
}
}
const parsedlayerList = await fetchAndParse(path);
const layerItems = parsedlayerList.querySelectorAll('item');
const layerItemLength = layerItems.length;
const data = layerItems[Math.floor(Math.random() * layerItemLength)];
const setup = setting || { campaign: '0000-00-00' };
var p = patOtaz_de.piwik;
// yeshide can be removed
if ( marOtaz_de.getCookie('tzi_paywall_payed') === 'yesHide') return;
p.trackEvent('TZI', 'layer', 'displayed');
const viaParamLayer = '&via=Layer,2021', viaParamLasche='&via=Lasche,2021';
const headline = data.querySelector('title').textContent;
const brottext = data.querySelector('teaser').textContent;
const primaryButtonText ='Ja, ich will';
const secondaryButtonText ='Schon dabei!';
const thxText ='Vielen Dank, dass Sie die taz unterstützen';
const closeText ='Gerade nicht';
const MinifiedTextDesktop =' Unterstützen Sie die taz jetzt freiwillig mit Ihrem Beitrag';
const MinifiedTextMobile =' Die taz unterstützen: ';
const picturePath = $(data).find('picture:first-of-type >pixmap[width="395"][mime-type="image/webp"]').attr('src') || undefined;
const artId = 1;
const fullFormURL = data.querySelector('meta url path[rel="speaking"]').getAttribute('href');
const param = data.querySelector('meta url fragment').textContent;
const form_et_link = fullFormURL + param + "&referrer=" + location.pathname?.replace(/[!/]/g, "_"); // remove ! and slash
const form_et_link_min = fullFormURL + param + "&referrer=" + location.pathname?.replace(/[!/]/g, "_"); // remove ! and slash
const container_fg = $(`
<div class="tziFgContainer2021  has-background-highlighted  has-background hasMultiply column pb-xlarge pb-0-mobile" x-data="" id="tzi-paywahl-fg">
<div class="inner full columns pv-0  ">
${picturePath !== undefined ? `
<div class="column is-5 tzi-picture-container">
<img class="tzi-picture" alt="taz zahl ich illustration " src="https://taz.de${picturePath}"></img>
</div>` : ``
}
<div class="column pv-0 is-justify-content-center">
<div class="is-flex is-align-items-center tzi-layer-head">
<span class="typo-portal-name   has-text-main  ">taz</span><span class="center icon no-hover xsmall pl-xxsmall taz_sub_logo_redhov   has-text-main   use-icon-by-unicode">&#xE80C</span>
<span class="typo-portal-name  has-text-main  ">zahl ich</span>
<div class="tzi-paywahl__close mgl-auto is-flex is-align-items-center icon-link-wrapper ">
<button class="typo-link text-link icon-link-text link " type="button" title="Banner schließen">
${closeText}
</button>
<button type="button" class=" 
mgl-small mgl-small-medium-mobile mgv-small-medium-mobile icon medium-large close_circle_redhov" aria-label="schließen"></button>
</div>
</div>
<p class="typo-head-extra-large   has-text-main  mgt-medium mgt-0-mobile tzi-fake-headline">${headline}</p>
<p class="article typo-subline tzi-lead-text mgt-medium mgt-small-mobile">${brottext}</p>
<ul class="article button-container is-align-items-center is-flex mgt-xlarge-xxlarge mgb-small-medium mgv-small-medium-mobile ">
<li class="tzi-paywahl__yes full-mobile pb-small-medium-mobile">
<a class="is-flex pulse-animation   has-background-main  is-align-items-center is-justify-content-center button primary-button  tzi-button mgt-0 has-text-white" title="Toll, dass Sie unabhängigen Journalismus möglich machen." href="${form_et_link + viaParamLayer}">
<span class=" icon medium no-hover use-icon-by-unicode has-text-white">&#xE808</span> <span style="text-wrap: nowrap;" class="tzi-shackle__yes" :class="isMobile ? 'typo-button-text' : 'typo-button-text-tzi'">
${primaryButtonText}
</span>
</a>
</li>
<li class="minimized__text mgl-0-mobile mgb-small-medium-mobile">
<span x-text="marOtaz_de.isMobile() ? '${MinifiedTextMobile}' : '${MinifiedTextDesktop}'" :class="isMobile? 'typo-teaser-text-bold mgr-small ' : 'typo-head-extrasmall mgr-medium'"></span>
</li>
<li class="thx__text full ">
<div class="is-flex is-align-items-center">
<span class="mgr-xsmall icon no-hover medium   has-text-main  use-icon-by-unicode ">&#xE808</span> <span class="typo-head-extrasmall"><strong>${thxText}</strong></span>
${/*  broken:
<button type="button" class="tzi-paywahl__close subscriber mgl-auto icon medium-large close_circle_redhov" aria-label="close">
</button>
*/''}
</div>
</li>
<li class="mgl-medium mgl-0-mobile tzi-paywahl__subscriber full-mobile mgt-0-mobile">
<a class="button    secondary-button tzi-button typo-button-text mgt-0" title="Danke!">
${secondaryButtonText}
</a>
</li>
<li class="tzi-paywahl__logo">
<a name="zahl ich" title="Infos über die freiwillige Unterstützung" target="_blank" href="/Werden-Sie-taz-Unterstuetzerin/!v=5bbaed78-169d-4b9b-a6e3-80fa0ac7ec47"></a>
</li>
</ul>
</div>
</div>
</div>
`);
// is not shown on notebooks (css)
const container_fgMin = $(`
<div id="tzi-paywahl-fg" class="tziFgContainer2021  has-background-highlighted  minimized  has-background hasMultiply alwaysmin">
<div class="inner full columns  ">
<div class="column pv-0 pv-0-mobile is-justify-content-center">
<ul class="article button-container ph-gap is-align-items-center is-flex">
<li class="tzi-paywahl__yes full-mobile">
<a class="is-flex  has-background-main   is-align-items-center is-justify-content-center button primary-button typo-button-text-tzi tzi-button mgt-0 has-text-white" title="Toll, dass Sie unabhängigen Journalismus möglich machen" href="${form_et_link + viaParamLayer}">
<span class="mgr-xsmall icon no-hover small use-icon-by-unicode has-text-white">&#xE808</span>
<span style="text-wrap: nowrap;" class="tzi-shackle__yes" :class="isMobile ? 'typo-button-text' : 'typo-button-text-tzi'">Ja, ich will</span>
</a>
</li>
<li class="minimized__text typo-head-extrasmall mgr-medium mgr-small-mobile" :class="isMobile? 'typo-teaser-text-bold ' : 'typo-head-extrasmall '">
<span x-text="marOtaz_de.isMobile() ? '${MinifiedTextMobile}' : '${MinifiedTextDesktop}'"></span>
</li>
</ul>
</div>
</div>
</div>
`);
// MAIN LOGIC IS HERE: LASCHE (dir pages) OR LAYER (articles)
if (isDirPage || isCMPtoDecideBeforeShowLayer ) {
// show full layer
container_fgMin.appendTo('body');
p.trackEvent('TZI', 'displayed', 'TZI_LASCHE_2021');
} else if (isArticlePage) {
// show minified layer (lasche)
container_fg.appendTo('body');
p.trackEvent('TZI', 'displayed', 'TZI_LAYER_2021');
}
$('.tzi-paywahl__yes', container_fg).on('click', function() {
if ($(this).closest('#tzi-paywahl-fg.minimized').length) {
p.trackEvent('TZI', 'clicked', 'TZI_LASCHE_Ja_2021');
} else {
p.trackEvent('TZI', 'clicked', 'TZI_LAYER_Ja_2021');
}
fadeOut(container_fg, function() {
fadeOut(container_bg);
});
});
$(document).on("click", ".tziFgContainer2021 .tzi-paywahl__close", function(event) {
p.trackEvent('TZI', 'clicked', 'LAYER-GERADE-NICHT_2021');
$('#tzi-paywahl-bg').css('display', 'none');
$('#tzi-paywahl-fg.tziFgContainer2021').addClass('transforming');
scrollay.maxTop = parseInt(window.outerHeight - heightOfMinimizedLayer);
$(container_fg.find('.tzi-paywahl__yes >.hint')).attr('href', form_et_link_min + viaParamLasche);
marOtaz_de.animate($('#tzi-paywahl-fg.tziFgContainer2021'), { top: scrollay.maxTop + "px" }, 500, function() {
marOtaz_de.animate($('#tzi-paywahl-fg.tziFgContainer2021'), { height: heightOfMinimizedLayer + "px" }, 200, function() {
$('#tzi-paywahl-fg.tziFgContainer2021').addClass('minimized closed').css('bottom', '0').css('top', 'auto');
$('#tzi-paywahl-fg.tziFgContainer2021').removeClass('transforming');
});
});
});
// can be removed now is same like "yes"
$(document).on("click", ".tziFgContainer2021 .tzi-paywahl__close.subscriber", function() {
document.cookie = "tzi_paywall_payed=yes; path=/; max-age=2592000";
fadeOut(container_fg, function() {
fadeOut(container_bg);
});
p.trackEvent('TZI', 'clicked', 'LAYER-THX-CLOSED_2021');
});
$(document).on("click", "#tzi-paywahl-fg .tzi-paywahl__subscriber", function() {
p.trackEvent('TZI', 'clicked', 'LAYER-SCHON-DABEI_2021');
document.cookie = "tzi_paywall_payed=yes; path=/; max-age=2592000";
scrollay.maxTop = parseInt(window.outerHeight - heightOfMinimizedLayer);
marOtaz_de.animate($('#tzi-paywahl-fg.tziFgContainer2021'), { top: scrollay.maxTop + "px" }, 500, function() {
marOtaz_de.animate($('#tzi-paywahl-fg.tziFgContainer2021'), { height: heightOfMinimizedLayer + "px" }, 200, function() {
$('#tzi-paywahl-fg.tziFgContainer2021').addClass('minimized closed thx fade-out').css('bottom', '0').css('top', 'auto');
$('#tzi-paywahl-fg.tziFgContainer2021.minimized.thx .thx__text').css('display', 'block');
});
});
});
var scrollay = {};
scrollay.initLayerHeight = $('#tzi-paywahl-fg.tziFgContainer2021').height();
scrollay.initLayerTop = $('#tzi-paywahl-fg.tziFgContainer2021').get(0).getBoundingClientRect().top;
scrollay.maxTop = parseInt(window.outerHeight - heightOfMinimizedLayer);
scrollay.contractionProgress;
scrollay.shift;
scrollay.scrollStartPos = document.documentElement.scrollTop;
var trackedClosedByScrolling = false;
};
document.addEventListener("DOMContentLoaded", function() {
const paywallPayed =  marOtaz_de.getCookie('tzi_paywall_payed');
if ((paywallPayed !== 'yes') && (!document.querySelector('#tzi-paywahl-fg'))) {
(async function () {
await marOtaz_de.tziLayerOldFrontend({'campaign': '2019-5-27' });
})();
}
});
;
// === global members
let p;
let globalTrackingParam = '';
if ( location.hash.indexOf('#!formfill:') ==0 ) globalTrackingParam = '&'+ location.hash.split(':')[1] ;
if ( location.hash.indexOf('#matomo:') ==0 ) globalTrackingParam = '&'+ location.hash.split(':')[1].replaceAll('pk_', 'mtm_');
// ====================================================== taz piwik library === //
window.patOtaz_de = window.patOtaz_de || new Object;
window.patOtaz_de.piwik = window.patOtaz_de.piwik || new (function (debug) {
// for a better readability
Array.prototype.clone = Array.prototype.slice;
// ES6 features
var hasHistoryApi = function() {
if ( history && history.replaceState instanceof Function )  return true;
return false;
};
// === private static members
var version = "patOtaz_de.piwik-0.6",
debug   =  debug || false;
// === logging in debug mode or from outside
var log = function(args) {
if ( debug && arguments.length ) {
var items = [];
Array.prototype.push.apply( items, arguments );
console.log( "PIWIK-LOG:", items );
}
return true;
};
// === clean up strings with a whitelist
var removeCharsWithWhitelist = function(validChars, inputString) {
var regex = new RegExp( '[^'+ validChars +']', 'g' );
return inputString.replace( regex, '' );
};
// === clean up #matomo url fragment and return matomo part
var removeHashParam = function(fragment_key) {
if ( fragment_key === undefined )  return '';
//   console.log('fragment_key ', fragment_key);
var href          = location.href;
var href_base     = href.split('#')[0];
var fragment_pos  = href.indexOf('#');
var fragment_full = href.substring( fragment_pos +1 );
if ( fragment_pos >0  &&  fragment_key.length >0 ) {
var fragment_regex = new RegExp( '(.*?)#?'+ fragment_key +':([^#]+)#?(.*)' );
var fragment_match = fragment_full.match( fragment_regex );
if ( fragment_match ) {
var fragment_this    = fragment_match[2];
var fragment_lhs     = fragment_match[1];
var fragment_rhs     = fragment_match[3];
var fragment_cleaned = ( fragment_lhs ? fragment_lhs : '' )+( fragment_lhs && fragment_rhs ? '#' : '' )+( fragment_rhs ? fragment_rhs : '' );
if ( fragment_this.length >0 ) {
//  var new_href = href_base +( !fragment_lhs && !fragment_rhs ? '' : '#' )+ fragment_cleaned;
//  if ( hasHistoryApi )  history.replaceState( {}, document.title, new_href );
return fragment_this;
}
}
}
return '';
};
// === track search results list page
var trackSiteSearch = function(keyword, category, count) {
var tmp = Array.prototype.clone.call( arguments );
tmp.unshift("trackSiteSearch");
_paq.push( tmp );
log( "search result", { keyword:keyword, category:category, count:count } );
return true;
};
// === track funnels
var trackFunnel = function(funnel, category) {
if ( funnel.length ) {
_paq.push([ 'trackEvent', category, 'Funnel', funnel ]);
log( "goal event", { funnel:funnel, category:category } );
}
return true;
};
// === track events
var trackEvent = function(category, action, name, value) {
var tmp = Array.prototype.clone.call( arguments );
tmp.unshift("trackEvent");
_paq.push( tmp );
log( "event", { category:category, action:action, name:name, value:value } );
return true;
}
// === debugging
log( version );
// === public interface
this.log              =log;
this.removeHashParam  =removeHashParam;
this.trackSiteSearch  =trackSiteSearch;
this.trackFunnel      =trackFunnel;
this.trackEvent       =trackEvent;
// this.trackPageType    =trackPageType;
})(false); // patOtaz_de.piwik
// ====================================================== piwik bucket === //
window._paq = window._paq || [];
// ====================================================== piwik setup === //
(function(){
// === private members
// changed by mar: web vs moby can not be serverside:
p = patOtaz_de.piwik,
o = {
version             : "0.6" // String
, piwik_script        : "https://taz.de/stats/piwik.php" // String
, page_id_enc         : "p4608" // String
, page_title_enc      : "p4608: taz.de"
, custom_url_enc      : "/!p4608" // String
, channel             : marOtaz_de.isMobile() ? 'moby' : 'web'
, area                : "Startseite" // String
, department          : "p4608" // String
, piwik_site_id       : "1"
, is_tag              : false // Boolean
, is_article          : false // Boolean
, has_tags            : false // Boolean -- careful: strange behaviour when using 'true' or 'false' as string. when serverside boolean is false than this will lead to Boolean(""), what is false in the end.
};
// === url decode strings
o.page_id         =decodeURIComponent( o.page_id_enc    );
o.page_title      =decodeURIComponent( o.page_title_enc );
o.custom_url      =decodeURIComponent( o.custom_url_enc );
// === page title
!function( node ) {
try {
var id    = node.getAttribute('data-id').toString()
, title = node.getAttribute('content').toString()
;
o.title = ( id.length >0 && title.length >0 ) ? id +": "+ title : "ZOMBIE";
} catch (err) {
o.title = "ZOMBIE";
console.log("ERROR-piwik: page title");
}
} ( document.head.querySelector( 'meta[ name="taz:title" ]' ) );
// === page url
o.url = location.href;
// === process and clean #matomo fragment, if present
o.url_fragment = decodeURIComponent( p.removeHashParam('matomo') );
if ( o.url_fragment.length >0 )  o.custom_url += '#'+ o.url_fragment;
// === track tags
if ( o.is_article && o.has_tags ) {
o.tag_list=[];
o.tag_list.toString = function() {
var string="";
// Get tag title and concat. So that no permutations are stored for the same set of tags, pull tag titles sorted by their id.
this.sort( function(a,b){ return ( parseInt(a.id) - parseInt(b.id) ) } ).forEach( function(tag){ string += tag.title +";" });
return string;
};
!function( tag_list ) {
var count=0;
try {
tag_list.forEach( function(tag) {
o.tag_list.push({
title     :tag.getAttribute('content')
,id       :tag.getAttribute('data-tag-id')
,category :tag.getAttribute('data-tag-category')? tag.getAttribute('data-tag-category'): "PLAIN-VANILLA"
});
count++;
});
if (count===0)  o.tag_list.push({ title:'ZOMBIE', id:0, category:'ZOMBIE' });
o.tag_list.string = o.tag_list.toString();
// console.log("tag_list", o.tag_list);
} catch(err) {
o.tag_list.push({ title:'ZOMBIE', id:0, category:'ZOMBIE' });
console.log("ERROR-piwik: tag_list");
}
}( document.head.querySelectorAll("meta[name='taz:tag']") );
}
// === setup piwik bucket
_paq.push([ "setCustomUrl"       ,o.custom_url ]);
_paq.push([ "setDocumentTitle"   ,o.page_title ]);
// debug parameters
//console.log('piwik setup', o);
_paq.push([ "setCustomVariable"  ,1 ,"channel"    ,o.channel    ,"page" ]);
_paq.push([ "setCustomVariable"  ,2 ,"area"       ,o.area       ,"page" ]);
_paq.push([ "setCustomVariable"  ,3 ,"department" ,o.department ,"page" ]);
if ( o.is_article && o.has_tags )  _paq.push([ "setCustomVariable", 4, "tag", o.tag_list.toString(), "page" ]);
_paq.push([ "trackPageView"      ]);
_paq.push([ "enableLinkTracking" ]);
// === load and setup piwik tracking
(function() {
var d=document
, g=d.createElement("script")
, s=d.getElementById("piwik-init")
;
_paq.push([ "setTrackerUrl" ,o.piwik_script  ]);
_paq.push([ "setSiteId"     ,o.piwik_site_id ]);
g.type   = "text/javascript";
g.id     = "piwik-loaded";
g.async  = true;
g.defer  = true;
g.src    = "https://taz.de/lib/share/js/piwik.js";
g.onload = function() { p.log( 'setup', o ) }
s.parentNode.insertBefore( g, s );
})();
// === tag tracking
if ( o.is_article && o.has_tags ) {
o.tag_list.forEach( function(tag) {    p.trackEvent( "TAG", "TAG-ARTICLE-"+ tag.category, "TAG - t"+ tag.id +": "+ tag.title ) });
}
if ( o.is_tag ) {
p.trackEvent( "TAG", "TAG-LANDINGPAGE", "TAG - "+ o.title );
}
// === debugging
//p.log( _paq );
// remove hash: see #16555
// this is running for the way from one url to another, but not for more
if( location.hash.indexOf('#!formfill:') ==0  || location.hash.indexOf('#matomo:') ==0 ) {
if( history && history.replaceState instanceof Function ){
history.replaceState({}, document.title, location.href.split('#')[0] );
}
}
})(); // piwik setup
// some events taken from olf frontend (custom-piwik.js)
function trackEventBasedOnCondition(selector, category, action, name) {
if (document.querySelector(selector)) {
//console.log('trackEvent', category, action, name);
p.trackEvent(category, action, name);
}
}
function trackClickEvent(selector, category, action, name) {
document.addEventListener("click", function(event) {
if (event.target.closest(selector)) {
p.trackEvent(category, action, name);
}
});
}
// you find this events matomo main dashbooard or under behaviour
document.addEventListener("DOMContentLoaded", function() {
setTimeout(function() {    trackEventBasedOnCondition("body.isArticle", 'element', 'displayed', document.querySelector("#tzi-paywahl-fg") ? 'ARTIKELAUFRUF_mit_Layer' : 'ARTIKELAUFRUF_ohne_Layer');}, 1000);
trackEventBasedOnCondition("body.isAuthor", 'element', 'displayed', 'AUTHOR-PAGE');
trackEventBasedOnCondition("body.isArticle", 'element', 'displayed', 'ARTIKELAUFRUF');
trackEventBasedOnCondition("body.isLongread", 'element', 'displayed', 'ARTIKELAUFRUF-LONGREAD');
trackClickEvent("#tzi-paywahl-fg .tzi-paywahl__yes", 'tzi', 'clicked', 'LAYER-JA');
trackClickEvent("#tzi-paywahl-fg .tzi-paywahl__close", 'tzi', 'clicked', 'LAYER-GERADE-NICHT');
trackClickEvent("#tzi-paywahl-fg .tzi-paywahl__subscriber", 'tzi', 'clicked', 'LAYER-SCHON-DABEI');
trackClickEvent("#tzi-shackle .tzi-shackle__yes", 'tzi', 'clicked', 'LASCHE-JA');
// if you want to track SM buttons
// trackClickEvent("#meineiddesbuttons", 'element', 'clicked', 'MEINE BEZEICHUNG FÜR DEN BUTTON IN MATOMO');
// bestr place for this?
document.body.classList.add('js-is-active');
// css helpers + feature detection  here
if (CSS.supports('selector(:has(*))')) {
document.body.classList.add('css-has-supported');
} else {
document.body.classList.add('css-has-not-supported');
}
const isSafari = navigator.vendor && navigator.vendor.indexOf('Apple') > -1 &&
navigator.userAgent &&
navigator.userAgent.indexOf('CriOS') == -1 &&
navigator.userAgent.indexOf('FxiOS') == -1;
if (isSafari)     document.documentElement.classList.add('is-safari');
})
;
document.addEventListener('DOMContentLoaded', () => {
const nav = document.querySelector('.header-nav-wing-inner[role="dialog"]');
if (!nav) return;
nav.addEventListener('keydown', function(e) {
// Prüfen ob das Dialog sichtbar ist
const isActive = nav.parentElement.classList.contains('is-active');
if (!isActive) return;
const focusableEls = nav.querySelectorAll('a, button, input, textarea, select, [tabindex]:not([tabindex="-1"])');
if (focusableEls.length === 0) return;
const firstEl = focusableEls[0];
const lastEl = focusableEls[focusableEls.length - 1];
if (e.key === 'Tab') {
if (e.shiftKey) {
if (document.activeElement === firstEl) {
e.preventDefault();
lastEl.focus();
}
} else {
if (document.activeElement === lastEl) {
e.preventDefault();
firstEl.focus();
}
}
}
});
});
;
function navbarScroll() {
return {
isOverflowing: false,
showLeftButton: false,
showRightButton: true,
updateScrollButtons() {
const navbar = this.$refs.navbarScrollbar;
if (!navbar) {
console.error('navbarScrollbar ref not found!');
return;
}
//debugging tags scrolling at the tags-navi aka Schwerpunkte-Navi / secondLevel navi
// console.log('=== debugging navbar starts here ===');
console.log('scrollWidth:', navbar?.scrollWidth, 'clientWidth:', navbar?.clientWidth);
this.isOverflowing = navbar.scrollWidth > navbar.clientWidth;
this.showLeftButton = navbar.scrollLeft > 0;
this.showRightButton = navbar.scrollLeft + navbar.clientWidth < navbar.scrollWidth;
//    console.log('isOverflowing:', this.isOverflowing,
//    'showLeft:', this.showLeftButton, 'showRight:', this.showRightButton);
//    console.log('========debugging ends here===========');
},
scrollLeft() {
const navbar = this.$refs.navbarScrollbar;
navbar.scrollBy({
left: -250,
behavior: 'smooth'
});
this.updateScrollButtons();
},
scrollRight() {
const navbar = this.$refs.navbarScrollbar;
navbar.scrollBy({
left: 250,
behavior: 'smooth'
});
this.updateScrollButtons();
}
}
}
;
// Vanilla-JS replacement for the former jQuery form ajaxification.
// Kept intentionally local to the form flow instead of rebuilding the old
// generic filOtaz_de.ajaxify / Href helper infrastructure.
// isTeaserLayout case: loadform is starting form alpine above when element is reached
if (
false === false && false
) {
loadForm();
}
function loadForm(storeCampaign) {
// Search is loaded together with the archive login form.
if ("taz_newsletter" === 'arc-search') return;
const formCode = "";
const formCodeAttr = "";
const formId = "taz_newsletter";
let formSel = '*[data-identifier="2wxl7tdcz"]';
let formUrl = '/!form=' + formId + ';x/';
// Archive login/search special case.
if (false === true) {
const match = /tazsid=([^;]*)/i.exec(document.cookie);
const value = match !== null ? match[1] : '0';
formUrl = '/!arcsid=' + value + '/';
}
// Archive: when src was supplied explicitly, call that complete URL.
if (false) formUrl = formId;
formUrl += '?node=1';
// Campaign forms are loaded after the campaign/bounty selection.
if (storeCampaign && storeCampaign.formId) {
formUrl = '/!form=' + storeCampaign.formId + ';x/?node=1' + storeCampaign.formActionAllParams();
formSel = '#' + cssEscape(storeCampaign.formId);
}
let param = (
marOtaz_de &&
typeof marOtaz_de.isMobile === 'function' &&
marOtaz_de.isMobile()
) ? '&mobile=1' : '&mobile=0';
if (formCodeAttr !== '') param += '&code=' + encodeURIComponent(formCodeAttr);
if (formCode !== '') param += '&code=' + encodeURIComponent(formCode);
// Comes from head_matomo.html.
if (typeof globalTrackingParam === 'string' && globalTrackingParam !== '') {
param += globalTrackingParam;
}
formUrl += param;
const target = document.querySelector(formSel);
if (!target) return;
const config = {
url: formUrl,
pick: 'fragment#form > *',
replacements: []
};

loadInitialForm(target, config);
}
function cssEscape(value) {
if (window.CSS && typeof window.CSS.escape === 'function') return window.CSS.escape(value);
return String(value).replace(/[^a-zA-Z0-9_-]/g, function (character) {
return '\\' + character;
});
}
async function loadInitialForm(target, config) {
try {
const responseDocument = await requestForm(config.url, 'GET');
mountResponse(target, responseDocument, config);
} catch (error) {
showRequestError(error);
}
}
function bindForm(form, config) {
let flying = false;
let lastSubmitter = null;
form.addEventListener('click', function (event) {
if (!(event.target instanceof Element)) return;
const submitter = event.target.closest(
'button[type="submit"], input[type="submit"], input[type="image"], button:not([type])'
);
if (submitter && form.contains(submitter)) lastSubmitter = submitter;
});
form.addEventListener('submit', async function (event) {
event.preventDefault();
if (flying) return;
flying = true;
const submitter = event.submitter || lastSubmitter || null;
const body = serializeForm(form, submitter);
const restore = blockForm(form);
try {
const responseDocument = await requestForm(config.url, 'POST', body);
const replaced = mountResponse(form, responseDocument, config);
// If the server returned no matching fragment, keep the old form usable.
if (!replaced) {
restore();
flying = false;
}
} catch (error) {
restore();
flying = false;
showRequestError(error);
}
});
}
function serializeForm(form, submitter) {
const params = new URLSearchParams();
const formData = new FormData(form);
formData.forEach(function (value, key) {
// jQuery.serialize() ignored file input contents as well.
if (typeof value === 'string') params.append(key, value);
});
// jQuery's old click handler added the clicked submit button to the form.
if (submitter && submitter.name && !submitter.disabled) {
params.append(submitter.name, submitter.value || '');
}
return params.toString();
}
async function requestForm(url, method, body) {
const requestUrl = new URL(url, window.location.href);
// Preserve the local-development behavior of the old helper.
if (window.location.hostname === 'localnode.taz.de') {
requestUrl.hostname = 'localnode.taz.de';
requestUrl.port = '';
}
/*
* IMPORTANT:
* Do not use requestUrl.searchParams here.
*
* The legacy Href implementation did not URL-encode parameter
* names and therefore preserved URLs such as:
*
*   ?/unsubscribe.php?reset=1&...
*
* URLSearchParams would turn this into:
*
*   ?%2Funsubscribe.php%3Freset=1&...
*
* Also preserve the old `xhr` flag semantics (`&xhr`, not
* `&xhr=true`).
*/
let requestHref = requestUrl.toString();
requestHref += requestUrl.search ? '&xhr' : '?xhr';
const options = {
method: method,
credentials: 'include',
cache: 'no-store',
headers: {}
};
if (requestUrl.origin === window.location.origin) {
options.headers['X-Requested-With'] = 'XMLHttpRequest';
}
if (method === 'POST') {
options.headers['Content-Type'] =
'application/x-www-form-urlencoded; charset=UTF-8';
options.body = body;
}
console.log(requestHref);
const response = await fetch(requestHref, options);
const responseText = await response.text();
if (!response.ok) {
const error = new Error(response.statusText || 'Fetch error');
error.status = response.status;
error.statusText = response.statusText;
throw error;
}
return new DOMParser().parseFromString(responseText, 'text/html');
}
function mountResponse(primaryTarget, responseDocument, config) {
const primaryReplaced = replaceFragment(
primaryTarget,
responseDocument,
config.pick,
true,
config
);
config.replacements.forEach(function (replacement) {
const target = document.querySelector(replacement.target);
if (!target) return;
replaceFragment(
target,
responseDocument,
replacement.pick,
replacement.bindForms !== false,
config
);
});
return primaryReplaced;
}
function replaceFragment(target, responseDocument, selector, shouldBindForms, config) {
const picked = Array.from(responseDocument.querySelectorAll(selector));
if (!picked.length || !target.parentNode) return false;
const marker = document.createComment('form ajax replacement');
const oldId = target.id;
const inserted = [];
const scripts = [];
target.parentNode.insertBefore(marker, target);
picked.forEach(function (sourceNode) {
const prepared = prepareNode(sourceNode);
scripts.push.apply(scripts, prepared.scripts);
if (!prepared.node) return;
if (oldId && prepared.node.nodeType === Node.ELEMENT_NODE) {
prepared.node.id = oldId;
}
marker.parentNode.insertBefore(prepared.node, marker);
inserted.push(prepared.node);
});
target.remove();
marker.remove();
if (shouldBindForms) {
inserted.forEach(function (node) {
if (node instanceof Element) bindFormsIn(node, config);
});
}
// Inline/external scripts contained in the returned fragment must be
// recreated to execute after insertion.
scripts.forEach(executeScript);
animateInsertedContent(inserted);
inserted.forEach(function (node) {
if (!(node instanceof Element)) return;
if (shouldBindForms) applyFormTabindex(node);
//dispatchChangeEvents(node);
showServerErrors(node);
});
return true;
}
function animateInsertedContent(nodes) {
if (window.matchMedia('(prefers-reduced-motion: reduce)').matches) return;
nodes.forEach(function (node) {
if (!(node instanceof Element) || typeof node.animate !== 'function') return;
node.animate(
[
{ opacity: 0, transform: 'translateY(5px)' },
{ opacity: 1, transform: 'translateY(0)' }
],
{ duration: 180, easing: 'ease-out' }
);
});
}
function bindFormsIn(root, config) {
if (root instanceof HTMLFormElement) bindForm(root, config);
root.querySelectorAll('form').forEach(function (form) {
bindForm(form, config);
});
}
function applyFormTabindex(root) {
const forms = [];
if (root instanceof HTMLFormElement) forms.push(root);
root.querySelectorAll('form').forEach(function (form) { forms.push(form); });
forms.forEach(function (form) {
form.querySelectorAll('input, textarea, select, button').forEach(function (control) {
if (isVisible(control)) control.tabIndex = 1;
});
});
}
function isVisible(element) {
return !!(element.offsetWidth || element.offsetHeight || element.getClientRects().length);
}
function prepareNode(node) {
const imported = document.importNode(node, true);
const scripts = [];
if (imported.nodeType === Node.ELEMENT_NODE && imported.matches('script')) {
scripts.push(imported);
return { node: null, scripts: scripts };
}
imported.querySelectorAll('script').forEach(function (script) {
scripts.push(script);
script.remove();
});
return { node: imported, scripts: scripts };
}
function executeScript(script) {
const executable = document.createElement('script');
Array.from(script.attributes).forEach(function (attribute) {
executable.setAttribute(attribute.name, attribute.value);
});
executable.textContent = script.textContent;
document.body.appendChild(executable);
}
function blockForm(form) {
const controls = Array.from(form.querySelectorAll('input, textarea, select, button'));
const sectBodies = Array.from(form.querySelectorAll('.sectbody'));
const controlState = controls.map(function (control) {
return { control: control, disabled: control.disabled };
});
const bodyState = sectBodies.map(function (body) {
return {
body: body,
opacity: body.style.opacity,
transition: body.style.transition
};
});
form.setAttribute('aria-busy', 'true');
sectBodies.forEach(function (body) {
body.style.transition = 'opacity 140ms ease';
body.style.opacity = '0.3';
});
controls.forEach(function (control) {
control.disabled = true;
});
return function restore() {
controlState.forEach(function (state) {
if (!state.control.isConnected) return;
state.control.disabled = state.disabled;
});
bodyState.forEach(function (state) {
if (!state.body.isConnected) return;
state.body.style.opacity = state.opacity;
state.body.style.transition = state.transition;
});
if (form.isConnected) form.removeAttribute('aria-busy');
};
}
/* function dispatchChangeEvents(element) {
element.dispatchEvent(new CustomEvent('TAZdomChange', { bubbles: true }));
element.dispatchEvent(new CustomEvent('TAZboxChange', { bubbles: true }));
}
*/
function showServerErrors(root) {
const errors = Array.from(root.querySelectorAll('.error'));
if (!errors.length) return;
const messages = errors
.map(function (error) { return error.textContent.trim(); })
.filter(Boolean);
errors.forEach(function (error) { error.remove(); });
if (messages.length) window.alert(messages.join('\n\n'));
}
function showRequestError(error) {
const status = error && error.status ? error.status + ' ' : '';
const message = error && (error.statusText || error.message)
? (error.statusText || error.message)
: 'Unbekannter Fehler';
console.error('Übertragungsfehler\n' + status + message);
}
