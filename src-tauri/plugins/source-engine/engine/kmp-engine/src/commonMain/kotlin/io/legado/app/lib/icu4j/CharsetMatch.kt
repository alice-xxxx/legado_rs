// © 2016 and later: Unicode, Inc. and others.
// License & terms of use: http://www.unicode.org/copyright.html
/*
 * ******************************************************************************
 * Copyright (C) 2005-2016, International Business Machines Corporation and    *
 * others. All Rights Reserved.                                                *
 * ******************************************************************************
 */
package io.legado.app.lib.icu4j

import io.legado.app.utils.textCharsetCodec

/** Detection result decoded through the engine's charset codec. */
class CharsetMatch internal constructor(
    detector: CharsetDetector,
    recognizer: CharsetRecognizer?,
    val confidence: Int,
    val name: String,
    val language: String?,
) : Comparable<CharsetMatch> {
    private val bytes = detector.fRawInput
    val string: String get() = getString(-1)

    internal constructor(detector: CharsetDetector, recognizer: CharsetRecognizer, confidence: Int) :
        this(detector, recognizer, confidence, recognizer.getName(), recognizer.getLanguage())

    fun getString(maxLength: Int): String {
        val decoded = textCharsetCodec(name.substringBefore("_rtl").substringBefore("_ltr")).decode(bytes)
        return if (maxLength > 0) decoded.take(maxLength) else decoded
    }

    override fun compareTo(other: CharsetMatch): Int = confidence.compareTo(other.confidence)
}
