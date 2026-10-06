#!/usr/bin/bash
toolforge build start https://github.com/magnusmanske/harvesttemplates
webservice restart
echo "OK"
