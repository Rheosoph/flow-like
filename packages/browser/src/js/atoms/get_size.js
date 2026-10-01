/* Derived from Chromium third_party/selenium-atoms/atoms.cc (GET_SIZE) @154.0.8037.92 (Selenium c0b8ff6), Copyright 2011-2014 Software Freedom Conservancy, Apache-2.0; modified by Rheosoph GmbH. See NOTICE. */
function(){return (function(){/*

 Copyright The Closure Library Authors.
 Copyright The Closure Compiler Authors.
 SPDX-License-Identifier: Apache-2.0
*/
var c=this||self;/*

 Copyright The Closure Library Authors.
 SPDX-License-Identifier: Apache-2.0
*/
function d(b,a){this.width=b;this.height=a}d.prototype.clone=function(){return new d(this.width,this.height)};d.prototype.aspectRatio=function(){return this.width/this.height};d.prototype.ceil=function(){this.width=Math.ceil(this.width);this.height=Math.ceil(this.height);return this};d.prototype.floor=function(){this.width=Math.floor(this.width);this.height=Math.floor(this.height);return this};d.prototype.round=function(){this.width=Math.round(this.width);this.height=Math.round(this.height);return this};
d.prototype.scale=function(b,a){this.width*=b;this.height*=typeof a==="number"?a:b;return this};function f(b){var a=b.offsetWidth,h=b.offsetHeight;if((a===void 0||!a&&!h)&&b.getBoundingClientRect){try{var e=b.getBoundingClientRect()}catch(l){e={left:0,top:0,right:0,bottom:0}}return new d(e.right-e.left,e.bottom-e.top)}return new d(a,h)};function g(b){b:{var a=b.nodeType==9?b:b.ownerDocument||b.document;if(a.defaultView&&a.defaultView.getComputedStyle&&(a=a.defaultView.getComputedStyle(b,null))){a=a.display||a.getPropertyValue("display")||"";break b}a=""}if((a||(b.currentStyle?b.currentStyle.display:null)||b.style&&b.style.display)!="none")b=f(b);else{a=b.style;var h=a.display,e=a.visibility,l=a.position;a.visibility="hidden";a.position="absolute";a.display="inline";b=f(b);a.display=h;a.position=l;a.visibility=e}return b}
var k=["se_exportedFunctionSymbol"],m=c;k[0]in m||typeof m.execScript=="undefined"||m.execScript("var "+k[0]);for(var n;k.length&&(n=k.shift());)k.length||g===void 0?m=m[n]&&m[n]!==Object.prototype[n]?m[n]:m[n]={}:m[n]=g;; return this.se_exportedFunctionSymbol.apply(null,arguments);}).apply(window, arguments);}
