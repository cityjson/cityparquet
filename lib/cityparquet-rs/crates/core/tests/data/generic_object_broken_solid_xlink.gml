<?xml version="1.0" encoding="utf-8"?>
<!-- hand-authored CityParquet test fixture (robustness).                      -->
<!-- A non-building object (Bridge) whose lod2Solid composes a surface by      -->
<!-- xlink:href that is defined nowhere. Unlike the same defect inside a       -->
<!-- boundedBy MultiSurface, which the reader warns about and drops, a solid   -->
<!-- cannot lose a face and stay the shape it claims to be, so this is fatal — -->
<!-- and the message has to say WHICH object, or an operator converting a      -->
<!-- corpus of thousands of files has nothing to search for.                   -->
<CityModel xmlns:xlink="http://www.w3.org/1999/xlink"
           xmlns:brid="http://www.opengis.net/citygml/bridge/2.0"
           xmlns:gml="http://www.opengis.net/gml"
           xmlns="http://www.opengis.net/citygml/2.0">
	<cityObjectMember>
		<brid:Bridge gml:id="the-bridge">
			<brid:lod2Solid>
				<gml:Solid>
					<gml:exterior>
						<gml:CompositeSurface>
							<gml:surfaceMember xlink:href="#missing"/>
						</gml:CompositeSurface>
					</gml:exterior>
				</gml:Solid>
			</brid:lod2Solid>
		</brid:Bridge>
	</cityObjectMember>
</CityModel>
