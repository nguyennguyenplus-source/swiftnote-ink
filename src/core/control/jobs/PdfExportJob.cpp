#include "PdfExportJob.h"

#include <memory>   // for unique_ptr, allocator
#include <string>   // for string
#include <utility>  // for move

#include "control/Control.h"               // for Control
#include "control/jobs/BaseExportJob.h"    // for BaseExportJob
#include "gui/swiftnote/SwiftBridge.h"
#include "model/Document.h"                // for Document
#include "pdf/base/XojPdfExport.h"         // for XojPdfExport
#include "pdf/base/XojPdfExportFactory.h"  // for XojPdfExportFactory
#include "util/PathUtil.h"                 // for clearExtensions
#include "util/i18n.h"                     // for _

PdfExportJob::PdfExportJob(Control* control, fs::path filepath, bool forceLightPaper):
        BaseExportJob(control, _("PDF Export")), forceLightPaper(forceLightPaper) {
    this->filepath = std::move(filepath);
}

PdfExportJob::~PdfExportJob() = default;

void PdfExportJob::run() {
    std::optional<swiftnote::DisplayState::ScopedExportOverride> exportOverride;
    if (this->forceLightPaper) {
        exportOverride.emplace(/*keepBlackboard=*/false, /*keepInkInvert=*/false);
    }

    Document* doc = control->getDocument();

    doc->lock_shared();
    std::unique_ptr<XojPdfExport> pdfe = XojPdfExportFactory::createExport(doc, control);
    doc->unlock_shared();

    if (testFilepath(this->filepath) && !pdfe->createPdf(this->filepath, false)) {
        // The path is valid but the export failed...
        this->errorMsg = pdfe->getLastError();
    }
    if (control->getWindow()) {
        // Will show putative error message
        callAfterRun();
    }
}
