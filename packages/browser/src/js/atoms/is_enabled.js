/* Derived from Chromium third_party/selenium-atoms/atoms.cc (IS_ENABLED) @154.0.8037.92 (Selenium c0b8ff6), Copyright 2011-2014 Software Freedom Conservancy, Apache-2.0; modified by Rheosoph GmbH. See NOTICE. */
function(){return (function(){/*

 Copyright The Closure Library Authors.
 Copyright The Closure Compiler Authors.
 SPDX-License-Identifier: Apache-2.0
*/
var c=this||self;/*

 Copyright The Closure Library Authors.
 SPDX-License-Identifier: Apache-2.0
*/
function e(a){for(var b=f,d=b.length,m=typeof b==="string"?b.split(""):b,h=0;h<d;h++)if(h in m&&a.call(void 0,m[h],h,b))return!0;return!1};function g(a){return k().indexOf(a)!=-1};function k(){var a=c.navigator;return a&&(a=a.userAgent)?a:""};function l(a){return(a=a.exec(k()))?a[1]:""}(g("iPhone")&&!g("iPod")&&!g("iPad")||g("iPad")||g("iPod")||g("Macintosh"))&&l(/CriOS\/([0-9.]+)/)||l(/Chrome\/([0-9.]+)/);function n(a){for(;a&&a.nodeType!=1;)a=a.previousSibling;return a}function p(a,b){for(var d=0;a;){if(b(a))return a;a=a.parentNode;d++}return null};function q(a,b){b&&typeof b!=="string"&&(b=b.toString());return a instanceof HTMLFormElement?!!a&&a.nodeType==1&&(!b||"FORM"==b):!!a&&a.nodeType==1&&(!b||a.tagName.toUpperCase()==b)};var f="BUTTON INPUT OPTGROUP OPTION SELECT TEXTAREA".split(" ");function r(a){return e(function(b){return q(a,b)})?a.disabled?!1:a.parentNode&&a.parentNode.nodeType==1&&q(a,"OPTGROUP")||q(a,"OPTION")?r(a.parentNode):!p(a,function(b){var d=b.parentNode;if(d&&q(d,"FIELDSET")&&d.disabled){if(!q(b,"LEGEND"))return!0;for(;b=b.previousElementSibling!==void 0?b.previousElementSibling:n(b.previousSibling);)if(q(b,"LEGEND"))return!0}return!1}):!0};var t=r,u=["se_exportedFunctionSymbol"],v=c;u[0]in v||typeof v.execScript=="undefined"||v.execScript("var "+u[0]);for(var w;u.length&&(w=u.shift());)u.length||t===void 0?v=v[w]&&v[w]!==Object.prototype[w]?v[w]:v[w]={}:v[w]=t;; return this.se_exportedFunctionSymbol.apply(null,arguments);}).apply(window, arguments);}
