#include "LinedBackgroundView.h"

#include <algorithm>  // for clamp, max, min
#include <memory>     // for allocator

#include "model/BackgroundConfig.h"               // for BackgroundConfig
#include "view/background/BackgroundView.h"       // for view
#include "view/background/RuledBackgroundView.h"  // for RuledBackgroundView

using namespace background_config_strings;
using namespace xoj::view;

LinedBackgroundView::LinedBackgroundView(double pageWidth, double pageHeight, Color backgroundColor,
                                         const BackgroundConfig& config):
        RuledBackgroundView(pageWidth, pageHeight, backgroundColor, config) {

    if (backgroundColor.isLight()) {
        vLineColor = getColorOr(config, CFG_FOREGROUND_COLOR_2, DEFAULT_V_LINE_COLOR);
    } else {
        vLineColor = getColorOr(config, CFG_ALT_FOREGROUND_COLOR_2, ALT_DEFAULT_V_LINE_COLOR);
    }

    config.loadValue(CFG_MARGIN, margin);
    if (margin < 0) {
        // A negative value puts the margin line on the right hand side
        margin += pageWidth;
    }

    int cornellFlag = 0;
    if (config.loadValue("cornell", cornellFlag)) {
        cornellMode = (cornellFlag != 0);
    }
}

void LinedBackgroundView::draw(cairo_t* cr) const {
    if (!cornellMode) {
        // Standard Xournal++ lined background: horizontal lines + full vertical margin
        RuledBackgroundView::draw(cr);

        cairo_save(cr);
        Util::cairo_set_source_rgbi(cr, vLineColor);
        cairo_set_line_width(cr, lineWidth);
        cairo_set_line_cap(cr, CAIRO_LINE_CAP_BUTT);
        cairo_move_to(cr, margin, 0);
        cairo_line_to(cr, margin, pageHeight);
        cairo_stroke(cr);
        cairo_restore(cr);
        return;
    }

    // Cornell Notes layout: clean header band (top 72pt), cue column (left margin),
    // ruled body notes area, and summary band (bottom 130pt).
    PlainBackgroundView::draw(cr);

    const double headerY = std::min(72.0, pageHeight * 0.14);
    const double summaryY = std::max(headerY + 80.0, pageHeight - 130.0);
    const double cueX = std::clamp(margin, 90.0, pageWidth * 0.42);

    // Horizontal ruled lines in the main notes column (right of cueX, between headerY and summaryY)
    cairo_save(cr);
    Util::cairo_set_source_rgbi(cr, foregroundColor);
    cairo_set_line_width(cr, lineWidth);
    cairo_set_line_cap(cr, CAIRO_LINE_CAP_BUTT);
    const double step = std::max(12.0, this->lineSpacing);
    for (double y = headerY + step; y < summaryY - 4.0; y += step) {
        cairo_move_to(cr, cueX + 8.0, y);
        cairo_line_to(cr, pageWidth - 20.0, y);
    }
    cairo_stroke(cr);

    // Structural Cornell dividers (header line, summary line, cue vertical line)
    Util::cairo_set_source_rgbi(cr, vLineColor);
    cairo_set_line_width(cr, lineWidth * 1.45);
    cairo_move_to(cr, 18.0, headerY);
    cairo_line_to(cr, pageWidth - 18.0, headerY);

    cairo_move_to(cr, 18.0, summaryY);
    cairo_line_to(cr, pageWidth - 18.0, summaryY);

    cairo_move_to(cr, cueX, headerY);
    cairo_line_to(cr, cueX, summaryY);
    cairo_stroke(cr);
    cairo_restore(cr);
}
