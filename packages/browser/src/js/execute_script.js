/* Derived from Chromium chrome/test/chromedriver/js/execute_script.js @154.0.8037.92, Copyright The Chromium Authors, BSD-3-Clause; modified by Rheosoph GmbH. See NOTICE. */
// Copyright 2019 The Chromium Authors
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

/**
* Execute the given script following the Execute-Script specification laid out
* in the W3C WebDriver spec, except for serialization/deserialization of args,
* which is handled by callFunction.
*
* @param {string} script The script to be executed.
* @param {!Array<*>} args Arguments to be passed to the script.
*/
function executeScript(script, args) {
  try {
    // Convert script (as a string) into an async function.
    const f = (new Function('return async function(){' + script + '}'))();
    const Promise = window.cdc_adoQpoasnfa76pfcZLmcfl_Promise || window.Promise;
    return Promise.resolve(f.apply(null, args));
  } catch (e) {
    return Promise.reject(e);
  }
}
